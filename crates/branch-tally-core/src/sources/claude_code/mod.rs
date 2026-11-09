//! The Claude Code source: finds `~/.claude/projects/<slug>/` directories for a repository and its
//! worktrees, and parses their JSONL transcripts into [`Record`]s.
//!
//! Format facts and their dates are in `docs/references/transcript_format.md`.
//!
//! # What is and is not counted
//!
//! Only lines carrying `message.usage` become records. A line without it (a user message, a
//! summary) is not a usage record and is **not counted anywhere**. A blank line is ignored. Every
//! usage record that is not emitted lands in exactly one [`ReadStats`] counter, checked in this
//! order: `synthetic`, `no_dedupe_key`, `duplicates_removed`. A line that is not valid JSON, or
//! whose declared fields have the wrong type (including a non-object `message`), counts as
//! `malformed_lines`.
//!
//! # Fallbacks
//!
//! - Dedupe key: `requestId`, else `uuid`. The `uuid` is stored in `Record::request_id`, with
//!   `Record::request_id_is_fallback` set, and `ReadStats::fallback_key_used` counts those emitted
//!   records (informational; duplicates and skipped records are not counted).
//! - `Record::session_id`: the record's `sessionId`, else the parent session (subagent file) or
//!   the file stem (main file).
//!
//! # Privacy
//!
//! Content fields are not deserialized, and errors carry paths only.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Harness, Record};
use crate::sources::{ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot};

mod parse;
mod slug;

pub use slug::slug_candidates;

use parse::{FileContext, read_file};
use slug::{Match, classify};

/// The Claude Code transcript reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCode;

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> SourceError + '_ {
    move |source| SourceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Entries of `dir` that are directories (`want_dirs`) or files, sorted by path for a
/// deterministic read order.
fn sorted_entries(dir: &Path, want_dirs: bool) -> Result<Vec<PathBuf>, SourceError> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        let kind = entry.file_type().map_err(io_error(&entry.path()))?;
        if kind.is_dir() == want_dirs {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

fn is_jsonl(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Source for ClaudeCode {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }

    fn discover(
        &self,
        projects_root: &Path,
        repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError> {
        if !projects_root.is_dir() {
            return Err(SourceError::ProjectsRootMissing {
                path: projects_root.to_path_buf(),
            });
        }
        let slugs = slug_candidates(repo.root());
        let mut roots = Vec::new();
        for dir in sorted_entries(projects_root, true)? {
            let name = file_name(&dir);
            let kind = slugs
                .iter()
                .find_map(|slug| classify(&name, slug))
                .map(|m| match m {
                    Match::Repository => RootKind::Repository,
                    Match::Worktree => RootKind::Worktree,
                });
            if let Some(kind) = kind {
                roots.push(TranscriptRoot::new(dir, kind));
            }
        }
        Ok(roots)
    }

    fn read(
        &self,
        roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError> {
        let mut stats = ReadStats::default();
        // One seen-set for the whole call: a replay in another root is still a duplicate.
        let mut seen = HashSet::new();
        for root in roots {
            let root_path = root.path();
            for file in sorted_entries(root_path, false)?
                .into_iter()
                .filter(|p| is_jsonl(p))
            {
                let stem = file
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let ctx = FileContext {
                    parent_session: None,
                    fallback_session: &stem,
                };
                read_file(&file, &ctx, &mut seen, &mut stats, sink)?;
            }
            for session_dir in sorted_entries(root_path, true)? {
                let subagents = session_dir.join("subagents");
                if !subagents.is_dir() {
                    continue;
                }
                let parent = file_name(&session_dir);
                for file in sorted_entries(&subagents, false)? {
                    if !is_jsonl(&file) || !file_name(&file).starts_with("agent-") {
                        continue;
                    }
                    let ctx = FileContext {
                        parent_session: Some(&parent),
                        fallback_session: &parent,
                    };
                    read_file(&file, &ctx, &mut seen, &mut stats, sink)?;
                }
            }
        }
        Ok(stats)
    }
}
