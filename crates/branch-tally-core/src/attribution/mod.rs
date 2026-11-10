//! Assigns each record the branch it belongs to.
//!
//! Ports worktree recovery from `load_records` and the strict filter or session claim from
//! `select` in `reference/token_report.py`, bug-for-bug (including the #6 worktree roll-up
//! behaviour). Consumes [`crate::model::Record`]; hands on [`crate::model::AttributedRecord`]
//! and [`crate::model::SelectedRecord`].

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::model::{
    AttributedRecord, BranchBasis, Origin, Record, SelectedRecord, Selection, SessionId,
};

/// The branch the reference reports for a record with no stamp.
const NONE: &str = "(none)";

/// What the reference's `worktree_branches` builds: worktree identity to the branch it holds.
///
/// Keyed two ways in one map, exactly as the reference does: the lowercased transcript slug of
/// each worktree path, and `worktree-<final path component>` (the harness-generated branch name
/// a subagent dispatch stamps). The main checkout's slug is kept so a `main` origin looks up the
/// same way the reference's does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeMap {
    by_key: HashMap<String, String>,
    main_slug: String,
}

impl WorktreeMap {
    /// Builds the map. `main_checkout` is the main checkout's path and `worktrees` every
    /// `(path, branch)` pair git reports (detached worktrees have no branch and are omitted by
    /// the caller). Paths should already be resolved (absolute, as git reports them).
    pub fn new<'a>(
        main_checkout: &Path,
        worktrees: impl IntoIterator<Item = (&'a Path, &'a str)>,
    ) -> Self {
        let mut by_key = HashMap::new();
        for (path, branch) in worktrees {
            by_key.insert(slug_for(path).to_lowercase(), branch.to_owned());
            by_key.insert(
                format!("worktree-{}", final_component(path)),
                branch.to_owned(),
            );
        }
        Self {
            by_key,
            main_slug: slug_for(main_checkout).to_lowercase(),
        }
    }

    /// The branch held under `key`, if any.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(String::as_str)
    }
}

/// The transcript slug of `path`: every character outside `[A-Za-z0-9]` becomes `-`
/// (`slug_for` in the reference).
#[must_use]
pub fn slug_for(path: &Path) -> String {
    path_text(path)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The path as text, without a Windows verbatim prefix (the reference's `resolve()` yields none).
fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .or_else(|| text.strip_prefix("//?/"))
        .unwrap_or(&text)
        .to_owned()
}

/// Python's `Path.name`: the last non-empty component, splitting on either separator so a
/// Windows path reads the same on every platform.
fn final_component(path: &Path) -> String {
    path_text(path)
        .rsplit(['/', '\\'])
        .find(|c| !c.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// `FEATURE_RE`: `^(feat|feature|fix|bugfix|hotfix|chore|release)/`.
fn is_feature(branch: &str) -> bool {
    [
        "feat/", "feature/", "fix/", "bugfix/", "hotfix/", "chore/", "release/",
    ]
    .iter()
    .any(|p| branch.starts_with(p))
}

/// Assigns each record a branch (`load_records`' branch step). Order is preserved; no record is
/// dropped. `strict` disables all recovery.
#[must_use]
pub fn attribute(
    records: impl IntoIterator<Item = Record>,
    map: &WorktreeMap,
    strict: bool,
) -> Vec<AttributedRecord> {
    records
        .into_iter()
        .map(|record| {
            let stamp = record
                .git_branch
                .as_deref()
                .filter(|b| !b.is_empty())
                .unwrap_or(NONE);
            let (branch, basis) = if strict {
                (stamp.to_owned(), BranchBasis::Stamped)
            } else {
                recover(&record, stamp, map)
            };
            AttributedRecord {
                record,
                branch,
                basis,
                subagent: None,
            }
        })
        .collect()
}

/// Non-strict recovery: a known `worktree-agent-` stamp, else `(none)` plus an origin lookup.
fn recover(record: &Record, stamp: &str, map: &WorktreeMap) -> (String, BranchBasis) {
    if stamp.starts_with("worktree-agent-")
        && let Some(branch) = map.get(stamp)
    {
        return (branch.to_owned(), BranchBasis::WorktreeRecovered);
    }
    if stamp == NONE {
        let key = match &record.origin {
            Some(Origin::Main) => Some(map.main_slug.clone()),
            Some(origin @ Origin::Worktree(_)) => Some(origin.label()),
            None => None,
        };
        if let Some(branch) = key.and_then(|k| map.get(&k)) {
            return (branch.to_owned(), BranchBasis::OriginRecovered);
        }
    }
    (stamp.to_owned(), BranchBasis::Stamped)
}

/// Picks the records belonging to `branch` (`select`). Order is preserved.
///
/// Strict keeps `assigned == branch`. Otherwise a session is claimed when its only work-shaped
/// branch is `branch`, and all its records are kept ([`Selection::SessionClaim`] unless the
/// record itself is on `branch`).
#[must_use]
pub fn select(records: Vec<AttributedRecord>, branch: &str, strict: bool) -> Vec<SelectedRecord> {
    let claimed: HashSet<SessionId> = if strict {
        HashSet::new()
    } else {
        let mut features: HashMap<&SessionId, HashSet<&str>> = HashMap::new();
        for r in &records {
            if is_feature(&r.branch) {
                features
                    .entry(&r.record.session_id)
                    .or_default()
                    .insert(&r.branch);
            }
        }
        features
            .into_iter()
            .filter(|(_, brs)| brs.len() == 1 && brs.contains(branch))
            .map(|(s, _)| s.clone())
            .collect()
    };
    records
        .into_iter()
        .filter_map(|attributed| {
            let kept_by = if attributed.branch == branch {
                Selection::Exact
            } else if claimed.contains(&attributed.record.session_id) {
                Selection::SessionClaim
            } else {
                return None;
            };
            Some(SelectedRecord {
                attributed,
                kept_by,
            })
        })
        .collect()
}
