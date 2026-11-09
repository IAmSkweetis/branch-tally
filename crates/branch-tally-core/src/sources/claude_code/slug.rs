//! Turning a repository path into the directory names Claude Code files its transcripts under.

use std::path::Path;

/// Prefix that follows a repository slug in a worktree session's directory name.
pub(super) const WORKTREE_INFIX: &str = "--claude-worktrees-";

/// Replaces every character outside `[A-Za-z0-9]` with `-`.
fn slugify(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The directory names a repository's own transcripts may use.
///
/// Claude Code derives the name from the session's cwd by replacing every character outside
/// `[A-Za-z0-9]` with `-`. Transcripts record the same Windows cwd as `D:\x` and as the Git Bash
/// form `/d/x`, so a path shaped like `/d/...` yields two candidates: the literal slug (a real
/// Unix path) and the Windows drive slug. A leading `\\?\` is stripped first. Compare the result
/// to directory names case-insensitively.
///
/// This works on the path's text, not the filesystem, so it behaves the same on every platform.
#[must_use]
pub fn slug_candidates(repo: &Path) -> Vec<String> {
    let text = repo.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\")
        .or_else(|| text.strip_prefix("//?/"))
        .unwrap_or(&text);

    let mut candidates = vec![slugify(text)];
    if let Some(rest) = text.strip_prefix('/') {
        let mut chars = rest.chars();
        if let Some(drive) = chars.next().filter(char::is_ascii_alphabetic) {
            let tail = chars.as_str();
            if tail.is_empty() || tail.starts_with('/') || tail.starts_with('\\') {
                let windows = format!("{}:{tail}", drive.to_ascii_uppercase());
                let slug = slugify(&windows);
                if !candidates.contains(&slug) {
                    candidates.push(slug);
                }
            }
        }
    }
    candidates
}

/// How a directory name relates to a repository slug.
pub(super) enum Match {
    Repository,
    Worktree,
}

/// Matches `dir_name` against one slug: exact, or slug plus the worktree infix and a non-empty
/// suffix. Never a bare prefix, so `repo` does not match `repo-cardgame`.
pub(super) fn classify(dir_name: &str, slug: &str) -> Option<Match> {
    if dir_name.eq_ignore_ascii_case(slug) {
        return Some(Match::Repository);
    }
    let stem_len = slug.len() + WORKTREE_INFIX.len();
    let stem = dir_name.get(..stem_len)?;
    let matches = stem
        .get(..slug.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(slug))
        && stem
            .get(slug.len()..)
            .is_some_and(|s| s.eq_ignore_ascii_case(WORKTREE_INFIX));
    (matches && dir_name.len() > stem_len).then_some(Match::Worktree)
}
