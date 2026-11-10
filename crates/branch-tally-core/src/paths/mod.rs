//! Path text normalisation and transcript slugs, shared by sources, attribution and git.
//!
//! Everything here works on a path's text, never the filesystem, so it behaves the same on every
//! platform and can be tested everywhere.

use std::path::Path;

/// Strips a Windows verbatim prefix from path text: `\\?\D:\x` and `//?/D:/x` become `D:\x` and
/// `D:/x`; `\\?\UNC\host\share\x` becomes `\\host\share\x` (and `//?/UNC/host/x` becomes
/// `//host/x`). Any other text is returned unchanged. The Git Bash `/d/x` form is left alone
/// because it may be a real Unix path; see [`windows_drive_form`].
#[must_use]
pub fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy();
    for (verbatim, unc, sep) in [(r"\\?\", r"UNC\", r"\\"), ("//?/", "UNC/", "//")] {
        if let Some(rest) = text.strip_prefix(verbatim) {
            return match rest.strip_prefix(unc) {
                Some(share) => format!("{sep}{share}"),
                None => rest.to_owned(),
            };
        }
    }
    text.into_owned()
}

/// The Windows drive form of a Git Bash path: `/d/x` becomes `D:/x` (separators after the drive
/// are kept as written). `None` when the text is not shaped `/<letter>` or `/<letter>/...`.
#[must_use]
pub fn windows_drive_form(text: &str) -> Option<String> {
    let rest = text.strip_prefix('/')?;
    let mut chars = rest.chars();
    let drive = chars.next().filter(char::is_ascii_alphabetic)?;
    let tail = chars.as_str();
    (tail.is_empty() || tail.starts_with('/') || tail.starts_with('\\'))
        .then(|| format!("{}:{tail}", drive.to_ascii_uppercase()))
}

/// Every character outside `[A-Za-z0-9]` replaced with `-`. Apply to normalised text.
#[must_use]
pub fn slugify(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The transcript slug of `path`: [`slugify`] of [`normalize`]. Literal; does not consider the
/// Git Bash drive form (use [`slug_candidates`] where that matters).
#[must_use]
pub fn slug(path: &Path) -> String {
    slugify(&normalize(path))
}

/// The slugs a path's transcript directory may carry, deduplicated, literal first. A `/d/...` path
/// yields the literal slug (it may be a real Unix path) and the `D:` drive slug. Callers compare
/// slugs case-insensitively.
#[must_use]
pub fn slug_candidates(path: &Path) -> Vec<String> {
    let text = normalize(path);
    let mut out = vec![slugify(&text)];
    if let Some(drive) = windows_drive_form(&text).map(|w| slugify(&w))
        && !out.contains(&drive)
    {
        out.push(drive);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> String {
        normalize(Path::new(s))
    }

    #[test]
    fn strips_verbatim_drive_prefix() {
        assert_eq!(n(r"\\?\D:\x"), r"D:\x");
        assert_eq!(n("//?/C:/x"), "C:/x");
    }

    #[test]
    fn unc_verbatim_becomes_plain_unc() {
        assert_eq!(n(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
        assert_eq!(n("//?/UNC/srv/share/x"), "//srv/share/x");
    }

    #[test]
    fn leaves_other_text_unchanged() {
        assert_eq!(n("/d/x"), "/d/x");
        assert_eq!(n("/repo"), "/repo");
        assert_eq!(n("c:/repo"), "c:/repo");
    }

    #[test]
    fn drive_form() {
        assert_eq!(windows_drive_form("/d/x").as_deref(), Some("D:/x"));
        assert_eq!(windows_drive_form("/d").as_deref(), Some("D:"));
        assert_eq!(windows_drive_form("/repo"), None);
        assert_eq!(windows_drive_form("c:/repo"), None);
    }

    #[test]
    fn slug_replaces_non_alphanumerics() {
        assert_eq!(
            slug(Path::new(r"D:\work spaces\my.repo_x")),
            "D--work-spaces-my-repo-x"
        );
        assert_eq!(slug(Path::new(r"\\?\D:\x")), "D--x");
        assert_eq!(slug(Path::new(r"\\?\UNC\srv\share\x")), "--srv-share-x");
        assert_eq!(slug(Path::new("//?/C:/x")), "C--x");
        assert_eq!(slug(Path::new("c:/repo")), "c--repo");
    }

    #[test]
    fn candidates_for_git_bash_path_have_both() {
        assert_eq!(
            slug_candidates(Path::new("/d/x")),
            vec!["-d-x".to_owned(), "D--x".to_owned()]
        );
    }

    #[test]
    fn candidates_for_other_paths_are_single() {
        assert_eq!(
            slug_candidates(Path::new("/repo")),
            vec!["-repo".to_owned()]
        );
        assert_eq!(
            slug_candidates(Path::new("c:/repo")),
            vec!["c--repo".to_owned()]
        );
        assert_eq!(
            slug_candidates(Path::new(r"D:\work spaces\my.repo_x")),
            vec!["D--work-spaces-my-repo-x".to_owned()]
        );
    }

    #[test]
    fn bare_drive_candidates() {
        assert_eq!(
            slug_candidates(Path::new("/d")),
            vec!["-d".to_owned(), "D-".to_owned()]
        );
    }
}
