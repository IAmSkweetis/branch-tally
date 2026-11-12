//! Release asset naming and `SHA256SUMS` verification.

use std::fmt::Write as _;

use super::version::Version;
use super::{UpdateError, scrub};

/// Name of the checksum file attached to every release.
pub const SUMS_NAME: &str = "SHA256SUMS";

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

/// Check `bytes` (the archive called `asset`) against the contents of a `SHA256SUMS` file.
///
/// # Errors
/// [`UpdateError::ChecksumMissing`], [`UpdateError::ChecksumMalformed`] or
/// [`UpdateError::ChecksumMismatch`]. Nothing is trusted unless the sums file lists `asset`.
pub fn verify_checksum(asset: &str, bytes: &[u8], sums: &str) -> Result<(), UpdateError> {
    let listed = sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == asset).then_some(digest)
    });
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
    let actual = ring::digest::digest(&ring::digest::SHA256, bytes);
    let actual_hex = actual
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    if actual_hex.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(UpdateError::ChecksumMismatch {
            asset: asset.to_string(),
        })
    }
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
