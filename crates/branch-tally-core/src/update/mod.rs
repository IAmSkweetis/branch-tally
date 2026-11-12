//! Self-update decisions: which release to install, whether to, and the verified install itself.
//!
//! Everything that touches the network, the filesystem or the terminal sits behind a trait
//! ([`ReleaseClient`], [`Installer`], [`Confirmer`]); the CLI supplies the real ones and core never
//! prints. Messages are plain, one line each.

mod assets;
mod version;

pub use assets::{
    SUMS_NAME, UNSUPPORTED_TARGET, archive_name, current_target, is_binary_entry,
    latest_version_from_sums, sha256_hex, verify_checksum,
};
pub use version::{DEV_VERSION, Version, is_dev_version};

/// The installer one-liners shown when `update` cannot run.
pub const INSTALL_HINT: &str = "install with: curl -LsSf https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh (Windows: irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex)";

/// Everything that can go wrong during an update.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// A version string was not `vMAJOR.MINOR.PATCH`.
    #[error("not a valid version: {0}")]
    InvalidVersion(String),
    /// No release is built for this platform.
    #[error("no release is built for target {0}")]
    UnsupportedTarget(String),
    /// The release tag or asset does not exist.
    #[error("release not found: {0}")]
    NotFound(String),
    /// The download failed.
    #[error("download failed: {0}")]
    Network(String),
    /// `SHA256SUMS` does not list the archive.
    #[error("{asset} is not listed in SHA256SUMS; nothing installed")]
    ChecksumMissing {
        /// Archive name.
        asset: String,
    },
    /// `SHA256SUMS` lists the archive with something that is not a SHA-256 digest.
    #[error("malformed checksum for {asset} in SHA256SUMS; nothing installed")]
    ChecksumMalformed {
        /// Archive name.
        asset: String,
    },
    /// The archive does not match its listed checksum.
    #[error("checksum mismatch for {asset}; nothing installed")]
    ChecksumMismatch {
        /// Archive name.
        asset: String,
    },
    /// Extracting or replacing the binary failed.
    #[error("install failed: {0}")]
    Install(String),
}

/// Why a release fetch failed. The client puts a short reason in `Failed`; core scrubs URLs out of
/// it before it reaches a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The tag or asset does not exist (HTTP 404).
    NotFound,
    /// Anything else: connection, TLS, status code, rate limit.
    Failed(String),
}

/// Reads GitHub Releases. The CLI implements this over HTTP.
pub trait ReleaseClient {
    /// The tag of the latest published release, for example `v1.2.3`.
    ///
    /// # Errors
    /// [`FetchError`] when it cannot be resolved.
    fn latest_tag(&self) -> Result<String, FetchError>;

    /// Download the asset called `asset` from the release `tag` (for example `v1.2.3`) as bytes.
    ///
    /// # Errors
    /// [`FetchError::NotFound`] for an unknown tag or asset, [`FetchError::Failed`] otherwise.
    fn download(&self, tag: &str, asset: &str) -> Result<Vec<u8>, FetchError>;
}

/// Replaces the running binary. The CLI implements this with its archive and filesystem crates.
pub trait Installer {
    /// Whether the directory holding the running binary can be written to. `Err` carries a short
    /// reason. Called before anything is downloaded.
    ///
    /// # Errors
    /// A reason string when the location is not writable.
    fn check_writable(&self) -> Result<(), String>;

    /// Extract the binary from the checksum-verified `archive` (named `asset`, which tells the
    /// implementation `.tar.gz` from `.zip`) and replace the running executable with it.
    ///
    /// # Errors
    /// A reason string when extraction or replacement fails. The old binary must stay usable.
    fn install(&self, asset: &str, archive: &[u8]) -> Result<(), String>;
}

/// Asks the user a yes/no question.
pub trait Confirmer {
    /// Show `prompt` and return whether the user agreed. Non-interactive implementations return
    /// `false`.
    fn confirm(&self, prompt: &str) -> bool;
}

/// What the user asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateOptions {
    /// Only report whether a newer version exists. Installs nothing, downloads nothing.
    pub check: bool,
    /// Install this release (`v1.2.3` or `1.2.3`) instead of the latest. Allows a downgrade.
    pub version: Option<String>,
    /// Skip the confirmation prompt.
    pub yes: bool,
    /// Reinstall even when already on the target version.
    pub force: bool,
}

/// Why `update` refused to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// This is a dev build with no release version.
    DevBuild,
    /// The binary's directory is not writable (reason from the installer).
    NotWritable(String),
}

/// What `update` did or would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Already on the target version and `--force` was not given.
    UpToDate {
        /// The running version.
        version: Version,
    },
    /// `--check` found a different target version. Nothing was changed.
    UpdateAvailable {
        /// The running version.
        current: Version,
        /// The version an update would install.
        available: Version,
    },
    /// The binary was replaced.
    Updated {
        /// Version before.
        from: Version,
        /// Version now installed.
        to: Version,
    },
    /// The user said no at the prompt. Nothing was changed.
    Declined {
        /// The running version.
        from: Version,
        /// The version that was offered.
        to: Version,
    },
    /// The update cannot run here. Nothing was changed.
    Refused(Refusal),
}

impl Outcome {
    /// One plain line for the user.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate { version } => {
                format!("branch-tally is up to date ({})", version.tag())
            }
            Self::UpdateAvailable { current, available } => format!(
                "update available: {} -> {}; run `branch-tally update` to install it",
                current.tag(),
                available.tag()
            ),
            Self::Updated { from, to } => {
                format!("updated branch-tally {} -> {}", from.tag(), to.tag())
            }
            Self::Declined { from, to } => {
                format!(
                    "update {} -> {} declined; nothing changed",
                    from.tag(),
                    to.tag()
                )
            }
            Self::Refused(Refusal::DevBuild) => {
                format!("this is a development build, so it cannot update itself; {INSTALL_HINT}")
            }
            Self::Refused(Refusal::NotWritable(reason)) => format!(
                "cannot update: the binary's directory is not writable ({}); {INSTALL_HINT}",
                scrub(reason)
            ),
        }
    }

    /// True for outcomes the CLI should exit non-zero on.
    #[must_use]
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Refused(_))
    }
}

/// Run `branch-tally update`.
///
/// `current` is the running binary's version string (it may be [`DEV_VERSION`]) and `target` its
/// target triple. Order of effects: dev-build refusal, then (unless `check`) a writability check,
/// then version resolution, then the prompt, and only then any download.
///
/// # Errors
/// [`UpdateError`] for an invalid or unknown version, an unsupported target, a failed download, a
/// failed checksum, or a failed install. In every error case the installed binary is untouched.
pub fn run(
    current: &str,
    target: &str,
    options: &UpdateOptions,
    client: &dyn ReleaseClient,
    installer: &dyn Installer,
    confirmer: &dyn Confirmer,
) -> Result<Outcome, UpdateError> {
    if is_dev_version(current) {
        return Ok(Outcome::Refused(Refusal::DevBuild));
    }
    let current = Version::parse(current)?;
    if !options.check
        && let Err(reason) = installer.check_writable()
    {
        return Ok(Outcome::Refused(Refusal::NotWritable(scrub(&reason))));
    }
    let pinned = options.version.as_deref().map(Version::parse).transpose()?;
    let wanted = match pinned {
        Some(version) => version,
        None => Version::parse(
            &client
                .latest_tag()
                .map_err(|e| fetch_error(&e, "latest release"))?,
        )?,
    };
    // Without a pin, an older "latest" is not an update; a pin may go either way.
    let nothing_to_do = if pinned.is_some() {
        wanted == current
    } else {
        wanted <= current
    };
    if options.check {
        return Ok(if nothing_to_do {
            Outcome::UpToDate { version: current }
        } else {
            Outcome::UpdateAvailable {
                current,
                available: wanted,
            }
        });
    }
    if nothing_to_do && !options.force {
        return Ok(Outcome::UpToDate { version: current });
    }
    let asset = archive_name(&wanted, target)?;
    if !options.yes {
        let prompt = format!("Update branch-tally {} -> {}?", current.tag(), wanted.tag());
        if !confirmer.confirm(&prompt) {
            return Ok(Outcome::Declined {
                from: current,
                to: wanted,
            });
        }
    }
    let tag = wanted.tag();
    let sums = client
        .download(&tag, SUMS_NAME)
        .map_err(|e| fetch_error(&e, &tag))?;
    let archive = client
        .download(&tag, &asset)
        .map_err(|e| fetch_error(&e, &tag))?;
    verify_checksum(&asset, &archive, &String::from_utf8_lossy(&sums))?;
    installer
        .install(&asset, &archive)
        .map_err(|reason| UpdateError::Install(scrub(&reason)))?;
    Ok(Outcome::Updated {
        from: current,
        to: wanted,
    })
}

fn fetch_error(error: &FetchError, what: &str) -> UpdateError {
    match error {
        FetchError::NotFound => UpdateError::NotFound(scrub(what)),
        FetchError::Failed(reason) => UpdateError::Network(scrub(reason)),
    }
}

/// Reduce free text from a client or installer to one line with every URL replaced, so a message
/// never carries a signed link or query-string token.
pub(crate) fn scrub(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(|word| if word.contains("://") { "<url>" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}
