//! Redaction stage: turns a [`Summary`] into one that hides identifying detail.
//!
//! Runs after the summary stage and before pricing and render. It never changes a number.

use std::collections::HashMap;

use crate::model::{AgentId, SessionId, SubagentRow, Summary, Workspace};

/// What to redact. `names: false` keeps agent and skill names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedactOptions {
    /// Replace agent and skill names with generic labels.
    pub names: bool,
}

impl Default for RedactOptions {
    fn default() -> Self {
        Self { names: true }
    }
}

/// Returns `summary` with identifying detail replaced. Totals, buckets and the row order that
/// costs index by are unchanged.
///
/// - Session ids become `s1`, `s2`, ... in ascending order of the original `first` timestamp
///   (ties keep their original order), and `summary.sessions` is reordered to match. `first` and
///   `last` shrink to `YYYY-MM-DD`; a value that is not date-shaped becomes `""` so no time of
///   day can leak. An origin other than `"main"` (a worktree slug) becomes `"worktree"`.
/// - With `opts.names`, agent and skill names become `agent-N` / `skill-N`, numbered in order of
///   first appearance walking `by_attribution`, then the subagent rows, then the sessions'
///   agent sets (sorted). One mapping serves the whole summary and is injective, so rows never
///   merge. `main` is kept. Without `opts.names` every name is kept verbatim.
/// - Always, regardless of `opts.names`: session ids, agent ids (`a1`, `a2`, ...) and worktree
///   names (`worktree-1`, ...) are replaced. Worktree names compare case-insensitively, as the
///   model does, so distinct-workspace counts are preserved.
///
/// Model ids and branch names are kept: they are not identifiers of a person or a path.
#[must_use]
pub fn redact(mut summary: Summary, opts: RedactOptions) -> Summary {
    let mut agents = Labels::new("agent");
    let mut skills = Labels::new("skill");

    if opts.names {
        for (key, _) in &mut summary.by_attribution {
            *key = relabel_attribution(key, &mut agents, &mut skills);
        }
        for row in &mut summary.subagents.rows {
            if !names_itself_by_id(row) {
                row.subagent = agents.get(&row.subagent);
            }
        }
    }

    let mut sessions = std::mem::take(&mut summary.sessions);
    // Stable: ties keep their original order.
    sessions.sort_by(|a, b| a.1.first.cmp(&b.1.first));
    summary.sessions = sessions
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut row))| {
            row.first = date_only(&row.first);
            row.last = date_only(&row.last);
            if row.origin != "main" {
                "worktree".clone_into(&mut row.origin);
            }
            if opts.names {
                row.agents = row.agents.iter().map(|a| agents.get(a)).collect();
            }
            (SessionId::new(format!("s{}", i + 1)), row)
        })
        .collect();

    let mut ids: HashMap<AgentId, AgentId> = HashMap::new();
    let mut worktrees: HashMap<String, String> = HashMap::new();
    for row in &mut summary.subagents.rows {
        if let Workspace::Worktree(name) = &row.workspace {
            let next = worktrees.len() + 1;
            let label = worktrees
                .entry(name.to_lowercase())
                .or_insert_with(|| format!("worktree-{next}"));
            row.workspace = Workspace::worktree(label.clone());
        }
        let by_id = names_itself_by_id(row);
        let old_tag = row.subagent.clone();
        let mut relabelled_tag = None;
        row.agent_ids = std::mem::take(&mut row.agent_ids)
            .into_iter()
            .map(|id| {
                let is_tag = by_id && id.as_str() == old_tag;
                let next = ids.len() + 1;
                let new = ids
                    .entry(id)
                    .or_insert_with(|| AgentId::new(format!("a{next}")))
                    .clone();
                if is_tag {
                    relabelled_tag = Some(new.as_str().to_owned());
                }
                new
            })
            .collect();
        if let Some(tag) = relabelled_tag {
            row.subagent = tag;
        }
    }

    summary.redacted = true;
    summary
}

/// Whether the row's `subagent` tag is one of its own agent ids. This is exactly how the
/// attribution stage tags a subagent with no stamped name (it falls back to the id), so such a
/// tag is an id, not a name, and is redacted whatever `names` says. A stamped name that happens
/// to equal its own id is indistinguishable and is treated as an id, which errs on the private
/// side.
fn names_itself_by_id(row: &SubagentRow) -> bool {
    row.agent_ids.iter().any(|id| id.as_str() == row.subagent)
}

/// `YYYY-MM-DD` taken from the front of an ISO timestamp, or `""` when it is not date-shaped.
fn date_only(ts: &str) -> String {
    let b = ts.as_bytes();
    let shaped = b.len() >= 10
        && b[..10].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
        && (b.len() == 10 || !b[10].is_ascii_digit());
    if shaped {
        ts[..10].to_owned()
    } else {
        String::new()
    }
}

fn relabel_attribution(key: &str, agents: &mut Labels, skills: &mut Labels) -> String {
    if key == "main" {
        "main".to_owned()
    } else if let Some(name) = key.strip_prefix("agent:") {
        format!("agent:{}", agents.get(name))
    } else {
        skills.get(key)
    }
}

/// An injective name-to-label map, numbering names in the order they are first asked for.
struct Labels {
    prefix: &'static str,
    map: HashMap<String, String>,
}

impl Labels {
    fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            map: HashMap::new(),
        }
    }

    fn get(&mut self, name: &str) -> String {
        let next = self.map.len() + 1;
        let prefix = self.prefix;
        self.map
            .entry(name.to_owned())
            .or_insert_with(|| format!("{prefix}-{next}"))
            .clone()
    }
}
