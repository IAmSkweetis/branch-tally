//! Choosing a price table (override, then cache, then bundled) and fetching a fresh one.

use std::io;
use std::path::{Path, PathBuf};

use crate::model::PriceTableSource;

use super::litellm::{LitellmImport, from_litellm};
use super::table::{PriceTable, PriceTableError};

/// Why the price feed could not be read. Carries a short reason, never a response body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeedError {
    /// The feed could not be reached or answered with an error.
    #[error("price feed unavailable: {0}")]
    Unavailable(String),
}

/// The network seam: the CLI implements it with HTTP, tests with a fake.
pub trait PriceFeed {
    /// The `LiteLLM` price JSON as text.
    ///
    /// # Errors
    ///
    /// [`FeedError`] when the feed cannot be read.
    fn fetch(&self) -> Result<String, FeedError>;
}

/// Reads candidate files, so tests can fake the filesystem.
pub trait TextSource {
    /// The file's text, or `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Any I/O failure other than not-found.
    fn read(&self, path: &Path) -> io::Result<Option<String>>;
}

/// [`TextSource`] over the real filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsText;

impl TextSource for FsText {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// A chosen table.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The table; its `source()` says which layer won.
    pub table: PriceTable,
}

/// Why resolution failed. Names the file by path, never its contents.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// A candidate exists but could not be read.
    #[error("cannot read price table {}: {kind}", path.display())]
    Unreadable {
        /// The file.
        path: PathBuf,
        /// The I/O error kind.
        kind: io::ErrorKind,
    },
    /// A candidate exists but is not a valid table.
    #[error("price table {} is invalid: {source}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        source: PriceTableError,
    },
}

/// Picks the first available of `override_path`, `cache_path`, then the bundled table.
///
/// A malformed or unreadable override or cache is an error, not a fall-through: silently pricing
/// with a different table than the user wrote would overstate certainty.
///
/// # Errors
///
/// [`ResolveError`] naming the offending file.
pub fn resolve(
    fs: &dyn TextSource,
    override_path: &Path,
    cache_path: &Path,
) -> Result<Resolved, ResolveError> {
    for (path, source) in [
        (override_path, PriceTableSource::Override),
        (cache_path, PriceTableSource::Cache),
    ] {
        let text = fs.read(path).map_err(|e| ResolveError::Unreadable {
            path: path.to_path_buf(),
            kind: e.kind(),
        })?;
        if let Some(text) = text {
            let table =
                PriceTable::parse(&text, source).map_err(|source| ResolveError::Malformed {
                    path: path.to_path_buf(),
                    source,
                })?;
            return Ok(Resolved { table });
        }
    }
    Ok(Resolved {
        table: PriceTable::bundled().clone(),
    })
}

/// Why `prices update` failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UpdateError {
    /// The feed could not be read.
    #[error(transparent)]
    Feed(#[from] FeedError),
    /// The feed was read but is not a usable `LiteLLM` price file.
    #[error("cannot convert price feed: {0}")]
    Convert(#[from] PriceTableError),
}

/// Fetches the feed and converts it to a cache-tagged table dated `as_of` (the caller's clock).
/// The CLI writes `import.table.to_toml()` to the cache path only when this succeeds, so a failed
/// update never replaces a good cache.
///
/// # Errors
///
/// [`UpdateError`] from the feed or the conversion.
pub fn fetch_table(feed: &dyn PriceFeed, as_of: &str) -> Result<LitellmImport, UpdateError> {
    let text = feed.fetch()?;
    Ok(from_litellm(&text, as_of)?)
}
