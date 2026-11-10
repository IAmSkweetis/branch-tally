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
//! - `Record::origin`: `Origin::Main` for records under the main checkout's root (including its
//!   `subagents/` files, as the reference does; see #6), `Origin::Worktree(<root dir name>)` for a
//!   worktree sibling root.
//!
//! # Read order
//!
//! Each root is read in one pass over every `.jsonl` at any depth, sorted like Python's
//! `sorted(root.rglob("*.jsonl"))`. Only `<session>/subagents/agent-*.jsonl` takes its
//! `parent_session` from the directory.
//!
//! # Dispatch links
//!
//! [`Source::dispatches`] reports one [`Dispatch`] per `(session, agent)` for every line whose
//! `toolUseResult` is an object with a non-empty string `agentId`, in main and subagent files
//! alike. Only `agentId` is deserialized from `toolUseResult`; its prompt, description and
//! output path are content. A line that is malformed or has no usable `agentId` yields nothing,
//! so the subagent it should have linked stays unlinked and is counted as unattributed
//! downstream.
//!
//! # Privacy
//!
//! Content fields are not deserialized, and errors carry paths only.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Dispatch, Harness, Origin, Record};
use crate::sources::{ReadStats, RepoIdentity, RootKind, Source, SourceError, TranscriptRoot};

mod parse;
mod slug;

use parse::{FileContext, read_dispatches, read_file};
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

/// Every `.jsonl` file under `dir` at any depth, with its path parts relative to the root.
/// Symlinked directories are not followed, as `Path.rglob` does not on the reference's Pythons.
fn collect_jsonl(
    dir: &Path,
    rel: &mut Vec<String>,
    out: &mut Vec<(PathBuf, Vec<String>)>,
) -> Result<(), SourceError> {
    for entry in fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(io_error(&path))?;
        rel.push(file_name(&path));
        if kind.is_dir() {
            collect_jsonl(&path, rel, out)?;
        } else if kind.is_file() && is_jsonl(&path) {
            out.push((path, rel.clone()));
        }
        rel.pop();
    }
    Ok(())
}

/// How `PurePath` orders two paths: part by part. Windows paths compare case-insensitively
/// (`PureWindowsPath` normcases to lower case); POSIX paths compare exactly. The reference runs on
/// the user's own platform, so this follows the platform branch-tally is built for. Strings
/// compare by code point (Python) which equals UTF-8 byte order (Rust).
fn sort_key(parts: &[String]) -> Vec<String> {
    if cfg!(windows) {
        parts.iter().map(|p| p.to_lowercase()).collect()
    } else {
        parts.to_vec()
    }
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
        let slugs = crate::paths::slug_candidates(repo.root());
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
            // Same rule as the reference: the main checkout's root is "main", any other root is
            // tagged by its own directory name. Subagent files share their root's origin.
            let origin = match root.kind() {
                RootKind::Repository => Origin::Main,
                RootKind::Worktree => Origin::worktree(file_name(root_path)),
            };
            // One pass per root over every `.jsonl` at any depth, in the reference's
            // `sorted(root.rglob("*.jsonl"))` order.
            for (file, rel) in sorted_jsonl(root_path)? {
                let stem = file_stem(&file);
                let ctx = file_context(&rel, &stem, &origin);
                read_file(&file, &ctx, &mut seen, &mut stats, sink)?;
            }
        }
        Ok(stats)
    }

    /// Dispatch links: lines carrying `toolUseResult.agentId` (the Agent tool's launch result),
    /// in main files and in subagent files alike (a subagent may launch its own). The dispatch
    /// session is the line's own `sessionId`, else the file's session as for records. A replay of
    /// the same (session, agent) pair is reported once, in read order.
    fn dispatches(&self, roots: &[TranscriptRoot]) -> Result<Vec<Dispatch>, SourceError> {
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for root in roots {
            let origin = Origin::Main; // unused by the dispatch pass
            for (file, rel) in sorted_jsonl(root.path())? {
                let stem = file_stem(&file);
                let ctx = file_context(&rel, &stem, &origin);
                read_dispatches(&file, &ctx, &mut seen, &mut found)?;
            }
        }
        Ok(found)
    }
}

/// Every `.jsonl` under `root` in the reference's `sorted(root.rglob("*.jsonl"))` order.
fn sorted_jsonl(root: &Path) -> Result<Vec<(PathBuf, Vec<String>)>, SourceError> {
    let mut files = Vec::new();
    collect_jsonl(root, &mut Vec::new(), &mut files)?;
    files.sort_by_cached_key(|f| sort_key(&f.1));
    Ok(files)
}

fn file_stem(file: &Path) -> String {
    file.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `<session>/subagents/agent-*.jsonl` takes its parent from the directory; every other file is
/// read like a main file (the record's own `sessionId` wins, else the file stem).
fn file_context<'a>(rel: &'a [String], stem: &'a str, origin: &'a Origin) -> FileContext<'a> {
    let parent = match rel {
        [session, dir, name] if dir == "subagents" && name.starts_with("agent-") => {
            Some(session.as_str())
        }
        _ => None,
    };
    FileContext {
        parent_session: parent,
        fallback_session: parent.unwrap_or(stem),
        origin,
    }
}
