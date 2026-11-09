//! The [`Source`] trait: how a harness's transcripts are found and read into [`Record`]s.
//!
//! A source is the first pipeline stage. It parses; it never decides a branch, and it never
//! resolves a home directory or reads an environment variable. The caller passes the harness's
//! projects root (`~/.claude/projects` for Claude Code, filled in by the CLI), so tests can point
//! a source straight at a fixture tree.
//!
//! # Reading shape
//!
//! [`Source::read`] pushes records into a `&mut dyn FnMut(Record)` sink instead of returning an
//! iterator. Removing duplicates needs state that spans files, and a sink lets the source own that
//! state without a self-referential iterator or boxed lifetimes. It also keeps memory flat, since
//! a caller can fold records into the ledger as they arrive.
//!
//! The trait is **object-safe**: no generics, no `Self` returns. `Box<dyn Source>` works.
//!
//! # Privacy
//!
//! [`SourceError`] and its `Display` text carry paths and counts only, never transcript content.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{Harness, Record};

pub mod claude_code;

/// The repository a source should find transcripts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    root: PathBuf,
}

impl RepoIdentity {
    /// Identifies a repository by the path of its main checkout.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The main checkout path. A source derives its own slug from this.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Why a discovered directory belongs to the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKind {
    /// The repository's own transcript directory.
    Repository,
    /// A transcript directory for one of the repository's worktrees.
    Worktree,
}

/// One directory of transcripts a source found for a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRoot {
    path: PathBuf,
    kind: RootKind,
}

impl TranscriptRoot {
    /// A discovered transcript directory.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, kind: RootKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether it is the repository's own or a worktree's.
    #[must_use]
    pub fn kind(&self) -> RootKind {
        self.kind
    }
}

/// What a read emitted and what it skipped, one counter per skip reason.
///
/// Governance II: nothing is dropped silently. The reference script drops all of these without a
/// word; here each is counted so the summary can report it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadStats {
    /// Transcript files opened.
    pub files_read: u64,
    /// Records passed to the sink.
    pub records_emitted: u64,
    /// Lines that were not valid JSON, or lacked a field the source needs.
    pub malformed_lines: u64,
    /// Usage records with no request id, so they cannot be de-duplicated safely.
    pub no_dedupe_key: u64,
    /// `<synthetic>` records, which are not real API calls.
    pub synthetic: u64,
    /// Records removed because an earlier record had the same dedupe key.
    pub duplicates_removed: u64,
    /// Records emitted with a fallback key in place of a request id. Informational: these are
    /// emitted, not skipped, so it is not part of [`ReadStats::skipped_total`].
    #[serde(default)]
    pub fallback_key_used: u64,
}

impl ReadStats {
    /// The number of usage records skipped for any reason.
    #[must_use]
    pub fn skipped_total(&self) -> u64 {
        self.malformed_lines + self.no_dedupe_key + self.synthetic + self.duplicates_removed
    }
}

/// Why discovery or reading failed outright. Per-line problems are counted in [`ReadStats`]
/// instead.
#[derive(Debug, Error)]
pub enum SourceError {
    /// The projects root does not exist or is not a directory.
    #[error("transcript projects root not found: {}", .path.display())]
    ProjectsRootMissing {
        /// The directory that was passed in.
        path: PathBuf,
    },
    /// A filesystem read failed.
    #[error("could not read {}: {source}", .path.display())]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// A harness's transcript reader.
pub trait Source {
    /// The harness this source reads.
    fn harness(&self) -> Harness;

    /// Finds the transcript directories under `projects_root` that belong to `repo`, including
    /// its worktree siblings.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if `projects_root` is missing or a directory cannot be listed.
    fn discover(
        &self,
        projects_root: &Path,
        repo: &RepoIdentity,
    ) -> Result<Vec<TranscriptRoot>, SourceError>;

    /// Reads every transcript under `roots`, passing each kept record to `sink`.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError`] if a file cannot be read. Bad lines are counted, not errors.
    fn read(
        &self,
        roots: &[TranscriptRoot],
        sink: &mut dyn FnMut(Record),
    ) -> Result<ReadStats, SourceError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RequestId, SessionId, Usage};

    struct Fake;

    impl Source for Fake {
        fn harness(&self) -> Harness {
            Harness::ClaudeCode
        }

        fn discover(
            &self,
            projects_root: &Path,
            _repo: &RepoIdentity,
        ) -> Result<Vec<TranscriptRoot>, SourceError> {
            if projects_root.as_os_str().is_empty() {
                return Err(SourceError::ProjectsRootMissing {
                    path: projects_root.to_path_buf(),
                });
            }
            Ok(vec![
                TranscriptRoot::new(projects_root.join("repo"), RootKind::Repository),
                TranscriptRoot::new(
                    projects_root.join("repo--claude-worktrees-a"),
                    RootKind::Worktree,
                ),
            ])
        }

        fn read(
            &self,
            roots: &[TranscriptRoot],
            sink: &mut dyn FnMut(Record),
        ) -> Result<ReadStats, SourceError> {
            let mut stats = ReadStats::default();
            for _ in roots {
                stats.files_read += 1;
                sink(Record::new(
                    Harness::ClaudeCode,
                    SessionId::new("s"),
                    RequestId::new("r"),
                    Usage::default(),
                ));
                stats.records_emitted += 1;
            }
            stats.malformed_lines = 1;
            stats.no_dedupe_key = 2;
            stats.synthetic = 3;
            stats.duplicates_removed = 4;
            Ok(stats)
        }
    }

    #[test]
    fn trait_is_object_safe_and_streams_records() {
        let source: Box<dyn Source> = Box::new(Fake);
        let repo = RepoIdentity::new("repo");
        assert_eq!(repo.root(), Path::new("repo"));
        let roots = source.discover(Path::new("projects"), &repo).unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].kind(), RootKind::Repository);
        assert_eq!(roots[1].kind(), RootKind::Worktree);
        assert_eq!(
            roots[1].path(),
            Path::new("projects").join("repo--claude-worktrees-a")
        );

        let mut seen = Vec::new();
        let stats = source.read(&roots, &mut |r| seen.push(r)).unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(stats.records_emitted, 2);
        assert_eq!(source.harness(), Harness::ClaudeCode);
    }

    #[test]
    fn skipped_total_sums_every_skip_reason() {
        let stats = ReadStats {
            files_read: 10,
            records_emitted: 20,
            malformed_lines: 1,
            no_dedupe_key: 2,
            synthetic: 3,
            duplicates_removed: 4,
            fallback_key_used: 0,
        };
        assert_eq!(stats.skipped_total(), 10);
    }

    #[test]
    fn fallback_key_used_is_informational_not_a_skip() {
        let stats = ReadStats {
            records_emitted: 5,
            fallback_key_used: 3,
            ..ReadStats::default()
        };
        assert_eq!(stats.skipped_total(), 0);
        let json = serde_json::to_string(&stats).unwrap();
        let back: ReadStats = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stats);
    }

    #[test]
    fn errors_name_paths_only() {
        let err = SourceError::ProjectsRootMissing {
            path: PathBuf::from("some/projects"),
        };
        assert_eq!(
            err.to_string(),
            format!(
                "transcript projects root not found: {}",
                Path::new("some/projects").display()
            )
        );
        let missing = Fake.discover(Path::new(""), &RepoIdentity::new("repo"));
        assert!(matches!(
            missing,
            Err(SourceError::ProjectsRootMissing { .. })
        ));
    }
}
