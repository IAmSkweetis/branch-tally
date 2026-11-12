//! Release versions: `vMAJOR.MINOR.PATCH`, and the dev-build marker.

use std::fmt;

use super::{UpdateError, scrub};

/// The version string a build without `BRANCH_TALLY_VERSION` reports.
pub const DEV_VERSION: &str = "0.0.0-dev";

/// A released version. Pre-release and build suffixes are not accepted: releases are plain triples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// Major component.
    pub major: u64,
    /// Minor component.
    pub minor: u64,
    /// Patch component.
    pub patch: u64,
}

impl Version {
    /// Parse `1.2.3` or `v1.2.3`.
    ///
    /// # Errors
    /// [`UpdateError::InvalidVersion`] when the text is not exactly three numeric parts.
    pub fn parse(text: &str) -> Result<Self, UpdateError> {
        let bad = || UpdateError::InvalidVersion(scrub(text));
        let body = text.strip_prefix('v').unwrap_or(text);
        let mut parts = body.split('.');
        let mut next = || -> Result<u64, UpdateError> {
            let part = parts.next().ok_or_else(bad)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            part.parse().map_err(|_| bad())
        };
        let (major, minor, patch) = (next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// The release tag form, `vMAJOR.MINOR.PATCH`.
    #[must_use]
    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// True when `version` is the dev-build marker (a build with no release version baked in).
#[must_use]
pub fn is_dev_version(version: &str) -> bool {
    version == DEV_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_leading_v() {
        let a = Version::parse("v1.2.3").unwrap();
        let b = Version::parse("1.2.3").unwrap();
        assert_eq!(a, b);
        assert_eq!((a.major, a.minor, a.patch), (1, 2, 3));
        assert_eq!(a.tag(), "v1.2.3");
        assert_eq!(a.to_string(), "1.2.3");
    }

    #[test]
    fn rejects_malformed_versions() {
        for bad in [
            "",
            "v",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "v1.2.3-rc1",
            "1..3",
            "-1.2.3",
            "0.0.0-dev",
            " 1.2.3",
            "+1.2.3",
        ] {
            assert!(
                matches!(Version::parse(bad), Err(UpdateError::InvalidVersion(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn compares_numerically_not_lexically() {
        assert!(Version::parse("1.10.0").unwrap() > Version::parse("1.9.9").unwrap());
        assert!(Version::parse("2.0.0").unwrap() > Version::parse("1.99.99").unwrap());
        assert!(Version::parse("0.3.1").unwrap() < Version::parse("0.3.2").unwrap());
    }

    #[test]
    fn recognises_the_dev_version() {
        assert!(is_dev_version("0.0.0-dev"));
        assert!(!is_dev_version("0.0.0"));
        assert!(!is_dev_version("1.0.0"));
    }
}
