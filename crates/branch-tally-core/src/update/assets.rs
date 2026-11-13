//! Release asset naming and `SHA256SUMS` verification.

use std::fmt::Write as _;

use super::version::Version;
use super::{UpdateError, scrub};

/// Name of the checksum file attached to every release.
pub const SUMS_NAME: &str = "SHA256SUMS";

/// The release asset holding the minisign signature of [`SUMS_NAME`].
pub const SIGNATURE_NAME: &str = "SHA256SUMS.minisig";

/// The target triples releases are built for.
const TARGETS: [&str; 6] = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
];

/// The release asset name for the prebuilt archive of `version` on `target`.
///
/// # Errors
/// [`UpdateError::UnsupportedTarget`] when no release is built for `target`.
pub fn archive_name(version: &Version, target: &str) -> Result<String, UpdateError> {
    let ext = if target.ends_with("-pc-windows-msvc") {
        "zip"
    } else {
        "tar.gz"
    };
    if !TARGETS.contains(&target) {
        return Err(UpdateError::UnsupportedTarget(scrub(target)));
    }
    Ok(format!("branch-tally-{}-{target}.{ext}", version.tag()))
}

/// The `(digest, file name)` pairs of a `SHA256SUMS` body. Lines without both fields are skipped;
/// a leading `*` (binary mode) on the name is dropped.
fn sums_entries(sums: &str) -> impl Iterator<Item = (&str, &str)> {
    sums.lines().filter_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        Some((digest, name))
    })
}

/// Check `bytes` (the archive called `asset`) against the contents of a `SHA256SUMS` file.
///
/// # Errors
/// [`UpdateError::ChecksumMissing`], [`UpdateError::ChecksumMalformed`] or
/// [`UpdateError::ChecksumMismatch`]. Nothing is trusted unless the sums file lists `asset`.
pub fn verify_checksum(asset: &str, bytes: &[u8], sums: &str) -> Result<(), UpdateError> {
    let listed = sums_entries(sums).find_map(|(digest, name)| (name == asset).then_some(digest));
    let Some(expected) = listed else {
        return Err(UpdateError::ChecksumMissing {
            asset: asset.to_string(),
        });
    };
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::ChecksumMalformed {
            asset: asset.to_string(),
        });
    }
    if sha256_hex(bytes).eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            asset: asset.to_string(),
        })
    }
}

/// Sentinel returned by [`current_target`] on a platform no release is built for.
/// [`archive_name`] reports it as [`UpdateError::UnsupportedTarget`].
pub const UNSUPPORTED_TARGET: &str = "unsupported";

/// The release target this binary was compiled for: the matching entry of the release list, or
/// [`UNSUPPORTED_TARGET`].
#[must_use]
pub fn current_target() -> &'static str {
    if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "musl"
    )) {
        TARGETS[0]
    } else if cfg!(all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "musl"
    )) {
        TARGETS[1]
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        TARGETS[2]
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        TARGETS[3]
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        TARGETS[4]
    } else if cfg!(all(
        target_arch = "aarch64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        TARGETS[5]
    } else {
        UNSUPPORTED_TARGET
    }
}

/// The version every archive in a `SHA256SUMS` body belongs to.
///
/// # Errors
/// [`UpdateError::InvalidVersion`] when no archive line carries a valid version, or when archive
/// lines name different versions.
pub fn latest_version_from_sums(sums: &str) -> Result<Version, UpdateError> {
    let mut found: Option<Version> = None;
    for (_, name) in sums_entries(sums) {
        let Some(rest) = name.strip_prefix("branch-tally-") else {
            continue;
        };
        let Some((tag, _)) = rest.split_once('-') else {
            continue;
        };
        if !tag.starts_with('v') {
            continue;
        }
        let Ok(version) = Version::parse(tag) else {
            continue;
        };
        match &found {
            Some(first) if *first != version => {
                return Err(UpdateError::InvalidVersion(
                    "SHA256SUMS lists archives of more than one version".to_string(),
                ));
            }
            _ => found = Some(version),
        }
    }
    found.ok_or_else(|| {
        UpdateError::InvalidVersion("SHA256SUMS lists no archive with a version".to_string())
    })
}

/// Whether the archive entry `path` is the `branch-tally` binary for `target`: a plain relative
/// path (either separator) whose last component is `branch-tally`, or `branch-tally.exe` for a
/// Windows target. Rejects `..`, absolute, drive-letter and leading-separator forms.
#[must_use]
pub fn is_binary_entry(path: &str, target: &str) -> bool {
    let bin = if target.contains("-windows-") {
        "branch-tally.exe"
    } else {
        "branch-tally"
    };
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    parts.last() == Some(&bin)
        && !parts.iter().any(|p| *p == ".." || p.contains(':'))
        && !path.starts_with(['/', '\\'])
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        ring::digest::digest(&ring::digest::SHA256, bytes)
            .as_ref()
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn current_target_is_listed_or_the_sentinel() {
        let t = current_target();
        if t == UNSUPPORTED_TARGET {
            assert!(matches!(
                archive_name(&v("1.2.3"), t),
                Err(UpdateError::UnsupportedTarget(_))
            ));
        } else {
            assert!(TARGETS.contains(&t));
            assert!(archive_name(&v("1.2.3"), t).is_ok());
        }
        #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "musl"))]
        assert_eq!(t, "x86_64-unknown-linux-musl");
        #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
        assert_eq!(t, UNSUPPORTED_TARGET);
    }

    #[test]
    fn version_comes_from_archive_lines_only() {
        let d = "0".repeat(64);
        let text = format!(
            "{d}  install.sh\n{d}  branch-tally-v1.2.3-x86_64-apple-darwin.tar.gz\n\
             {d} *branch-tally-v1.2.3-x86_64-pc-windows-msvc.zip\r\n"
        );
        assert_eq!(latest_version_from_sums(&text).unwrap(), v("1.2.3"));
    }

    #[test]
    fn sums_without_a_valid_archive_version_is_an_error() {
        let d = "0".repeat(64);
        for text in [
            String::new(),
            format!("{d}  install.sh\n"),
            format!("{d}  branch-tally-vx.y-linux.tar.gz\n"),
            format!("{d}  branch-tally-1.2.3-linux.tar.gz\n"),
        ] {
            assert!(
                matches!(
                    latest_version_from_sums(&text),
                    Err(UpdateError::InvalidVersion(_))
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn sums_disagreeing_on_version_is_an_error() {
        let d = "0".repeat(64);
        let text =
            format!("{d}  branch-tally-v1.2.3-a.tar.gz\n{d}  branch-tally-v1.2.4-b.tar.gz\n");
        assert!(matches!(
            latest_version_from_sums(&text),
            Err(UpdateError::InvalidVersion(_))
        ));
    }

    #[test]
    fn binary_entry_accepts_plain_and_nested_paths() {
        let unix = "x86_64-unknown-linux-musl";
        let win = "x86_64-pc-windows-msvc";
        assert!(is_binary_entry("branch-tally", unix));
        assert!(is_binary_entry(
            "branch-tally-v1.2.3-x86_64-unknown-linux-musl/branch-tally",
            unix
        ));
        assert!(is_binary_entry("branch-tally.exe", win));
        assert!(is_binary_entry("dir\\branch-tally.exe", win));
        assert!(is_binary_entry(
            "branch-tally-v1.2.3-x86_64-pc-windows-msvc/branch-tally.exe",
            win
        ));
    }

    #[test]
    fn binary_entry_rejects_unsafe_and_wrong_platform_paths() {
        let unix = "aarch64-apple-darwin";
        let win = "aarch64-pc-windows-msvc";
        for p in [
            "../x/branch-tally",
            "/abs/branch-tally",
            "\\abs\\branch-tally",
            "a/../branch-tally",
            "branch-tally-extra",
            "branch-tally/",
            "other/branch-tally.exe",
        ] {
            assert!(!is_binary_entry(p, unix), "{p}");
        }
        for p in [
            "C:branch-tally.exe",
            "C:\\x\\branch-tally.exe",
            "../x/branch-tally.exe",
            "/abs/branch-tally.exe",
            "branch-tally",
        ] {
            assert!(!is_binary_entry(p, win), "{p}");
        }
        assert!(!is_binary_entry("branch-tally.exe", unix));
        assert!(!is_binary_entry("x/branch-tally", win));
    }

    #[test]
    fn names_all_six_targets() {
        let cases = [
            ("x86_64-unknown-linux-musl", "tar.gz"),
            ("aarch64-unknown-linux-musl", "tar.gz"),
            ("x86_64-apple-darwin", "tar.gz"),
            ("aarch64-apple-darwin", "tar.gz"),
            ("x86_64-pc-windows-msvc", "zip"),
            ("aarch64-pc-windows-msvc", "zip"),
        ];
        for (target, ext) in cases {
            assert_eq!(
                archive_name(&v("1.2.3"), target).unwrap(),
                format!("branch-tally-v1.2.3-{target}.{ext}")
            );
        }
    }

    #[test]
    fn unknown_target_is_an_error() {
        let err = archive_name(&v("1.2.3"), "riscv64-unknown-linux-gnu").unwrap_err();
        assert!(matches!(err, UpdateError::UnsupportedTarget(_)));
    }

    #[test]
    fn accepts_matching_checksum_in_both_sums_formats() {
        let data = b"archive bytes";
        let h = hex(data);
        let text = format!("{} other.tar.gz\n{h}  a.tar.gz\n", "0".repeat(64));
        assert!(verify_checksum("a.tar.gz", data, &text).is_ok());
        let star = format!("{}  *a.tar.gz\r\n", h.to_uppercase());
        assert!(verify_checksum("a.tar.gz", data, &star).is_ok());
    }

    #[test]
    fn mismatch_is_an_error() {
        let text = format!("{}  a.tar.gz\n", hex(b"other"));
        assert!(matches!(
            verify_checksum("a.tar.gz", b"archive bytes", &text),
            Err(UpdateError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn unlisted_asset_is_an_error() {
        let text = format!("{}  b.tar.gz\n", hex(b"archive bytes"));
        assert!(matches!(
            verify_checksum("a.tar.gz", b"archive bytes", &text),
            Err(UpdateError::ChecksumMissing { .. })
        ));
        assert!(matches!(
            verify_checksum("a.tar.gz", b"archive bytes", ""),
            Err(UpdateError::ChecksumMissing { .. })
        ));
    }

    #[test]
    fn malformed_checksum_is_an_error() {
        for bad in ["abc", &"g".repeat(64), &"a".repeat(63)] {
            let text = format!("{bad}  a.tar.gz\n");
            assert!(
                matches!(
                    verify_checksum("a.tar.gz", b"x", &text),
                    Err(UpdateError::ChecksumMalformed { .. })
                ),
                "{bad}"
            );
        }
    }
}
