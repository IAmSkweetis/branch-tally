//! Matching Claude Code projects directory names against a repository slug. The slug candidates
//! themselves come from [`crate::paths::slug_candidates`].

/// Prefix that follows a repository slug in a worktree session's directory name.
pub(super) const WORKTREE_INFIX: &str = "--claude-worktrees";

/// How a directory name relates to a repository slug.
pub(super) enum Match {
    Repository,
    Worktree,
}

/// Matches `dir_name` against one slug: exact, or slug plus the worktree infix. Never a bare
/// prefix, so `repo` does not match `repo-cardgame`.
pub(super) fn classify(dir_name: &str, slug: &str) -> Option<Match> {
    if dir_name.eq_ignore_ascii_case(slug) {
        return Some(Match::Repository);
    }
    // Same rule as `transcript_roots` in the reference: the slug plus `--claude-worktrees`, with
    // no requirement for a trailing `-` or a suffix.
    let stem_len = slug.len() + WORKTREE_INFIX.len();
    let stem = dir_name.get(..stem_len)?;
    let matches = stem
        .get(..slug.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(slug))
        && stem
            .get(slug.len()..)
            .is_some_and(|s| s.eq_ignore_ascii_case(WORKTREE_INFIX));
    matches.then_some(Match::Worktree)
}
