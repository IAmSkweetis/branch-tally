//! The `prices` subcommand and the price-table lookup `report` shares with it.
//!
//! Lookup order, first found wins whole: `.tally/prices.toml` in the main checkout (override),
//! the per-user cache written by `prices update`, then the table built into the binary.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use branch_tally_core::git::{Git, SystemGit, normalize_path};
use branch_tally_core::ledger::{LedgerError, TallyDir};
use branch_tally_core::model::PriceTableSource;
use branch_tally_core::pricing::{
    FeedError, FsText, PriceFeed, PriceTable, TextSource, UpdateError, fetch_table, resolve,
};

use crate::cli::{PricesCommand, ShowArgs};

/// The `LiteLLM` model price list, raw from GitHub `main`.
const FEED_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Test-only: replaces [`FEED_URL`] so end-to-end tests can point `prices update` at a loopback
/// server. Not documented for users and not a supported interface.
const FEED_URL_ENV: &str = "BRANCH_TALLY_PRICES_URL";

/// The largest feed accepted. The real file is a few MB; this only stops a runaway response.
const MAX_FEED_BYTES: u64 = 64 * 1024 * 1024;

/// The override's file name inside `.tally/`.
const PRICES_FILE: &str = "prices.toml";

/// Reads the override through [`TallyDir`] (no symlink is followed, `.tally/` is never created,
/// #94) and every other path, the per-user cache, as a plain file.
struct GuardedText<'a> {
    main: &'a Path,
    override_path: &'a Path,
    /// The core refusal text, kept because `ResolveError::Unreadable` carries only the I/O kind.
    refusal: std::cell::RefCell<Option<String>>,
}

impl TextSource for GuardedText<'_> {
    fn read(&self, path: &Path) -> std::io::Result<Option<String>> {
        if path != self.override_path {
            return FsText.read(path);
        }
        let refused = |e: LedgerError| {
            *self.refusal.borrow_mut() = Some(e.to_string());
            std::io::Error::other("refused")
        };
        let Some(tally) = TallyDir::open(self.main).map_err(refused)? else {
            return Ok(None);
        };
        let Some(bytes) = tally.read(PRICES_FILE).map_err(refused)? else {
            return Ok(None);
        };
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"))
    }
}

/// Where the override and cache files are looked for.
pub(crate) struct Locations {
    /// `<main checkout>/.tally/prices.toml`.
    pub(crate) override_path: PathBuf,
    /// The main checkout, whose `.tally/` the override is read through without following links.
    main: PathBuf,
    /// The per-user cache file; `None` when the platform's cache directory cannot be determined.
    pub(crate) cache_path: Option<PathBuf>,
}

impl Locations {
    pub(crate) fn for_repo(repo: &Path) -> Self {
        let main = SystemGit.main_checkout(repo);
        Self {
            override_path: main.join(".tally").join(PRICES_FILE),
            main,
            cache_path: cache_path(),
        }
    }
}

/// Picks the table for a report. `Err` is a one-line message naming the offending file.
pub(crate) fn resolve_table(locations: &Locations) -> Result<PriceTable, String> {
    // With no cache directory the cache layer is skipped: an empty path is never found.
    let cache = locations.cache_path.as_deref().unwrap_or(Path::new(""));
    let fs = GuardedText {
        main: &locations.main,
        override_path: &locations.override_path,
        refusal: std::cell::RefCell::new(None),
    };
    resolve(&fs, &locations.override_path, cache)
        .map(|resolved| resolved.table)
        .map_err(|e| one_line(&fs.refusal.take().unwrap_or_else(|| e.to_string())))
}

/// F-03: a repository price override replaces the whole table and is unauthenticated, so any run
/// that prices with it says so, on stderr only (stdout may be JSON).
pub(crate) fn note_override(table: &PriceTable) {
    if table.source() == PriceTableSource::Override {
        crate::report::stderr(
            "note: using the repository price override (.tally/prices.toml); prices are not the \
             bundled or cached table",
        );
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The per-user cache file from the real environment.
pub(crate) fn cache_path() -> Option<PathBuf> {
    cache_path_from(|key| std::env::var_os(key))
}

/// The cache file under the platform's cache directory: Windows `%LOCALAPPDATA%`, macOS
/// `$HOME/Library/Caches`, elsewhere `$XDG_CACHE_HOME` or `$HOME/.cache`. Empty values count as
/// unset, as the XDG specification says for relative or empty paths.
fn cache_path_from(get: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let non_empty = |key: &str| get(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        non_empty("LOCALAPPDATA")?
    } else if cfg!(target_os = "macos") {
        non_empty("HOME")?.join("Library").join("Caches")
    } else {
        match non_empty("XDG_CACHE_HOME").filter(|p| p.is_absolute()) {
            Some(dir) => dir,
            None => non_empty("HOME")?.join(".cache"),
        }
    };
    Some(base.join("branch-tally").join("prices.toml"))
}

/// Entry point for `branch-tally prices …`. `Ok` carries the text for stdout.
pub(crate) fn run(command: &PricesCommand) -> Result<String, String> {
    match command {
        PricesCommand::Update => {
            let cache = cache_path().ok_or(
                "Could not determine the cache directory (set LOCALAPPDATA, XDG_CACHE_HOME or HOME)",
            )?;
            let url = std::env::var(FEED_URL_ENV).unwrap_or_else(|_| FEED_URL.to_owned());
            let as_of = today_utc(SystemTime::now());
            let outcome = update(&HttpFeed { url }, &as_of, &cache)?;
            Ok(outcome.describe(&cache))
        }
        PricesCommand::Show(args) => show(args),
    }
}

fn show(args: &ShowArgs) -> Result<String, String> {
    let locations = Locations::for_repo(&normalize_path(&args.repo));
    let table = resolve_table(&locations)?;
    let path = match table.source() {
        PriceTableSource::Override => locations.override_path.display().to_string(),
        PriceTableSource::Cache => locations
            .cache_path
            .as_deref()
            .map_or_else(String::new, |p| p.display().to_string()),
        PriceTableSource::Bundled => "(built into the binary)".to_owned(),
    };
    Ok(format!(
        "source: {}\nas of: {}\nmodels: {}\npath: {path}\n",
        source_name(table.source()),
        table.as_of(),
        table.models().len(),
    ))
}

fn source_name(source: PriceTableSource) -> &'static str {
    match source {
        PriceTableSource::Bundled => "bundled",
        PriceTableSource::Cache => "cache",
        PriceTableSource::Override => "override",
    }
}

/// What a successful update did.
#[derive(Debug, PartialEq)]
pub(crate) struct Updated {
    pub(crate) as_of: String,
    pub(crate) models: usize,
    pub(crate) skipped: Vec<String>,
}

impl Updated {
    fn describe(&self, path: &Path) -> String {
        let mut out = format!(
            "Wrote {}\nas of: {}\nmodels: {}\n",
            path.display(),
            self.as_of,
            self.models
        );
        if self.skipped.is_empty() {
            out.push_str("skipped: none\n");
        } else {
            let _ = writeln!(
                out,
                "skipped {} model(s) without a usable price: {}",
                self.skipped.len(),
                self.skipped.join(", ")
            );
        }
        out
    }
}

/// Fetches, converts, and only then replaces the cache file (temp file and rename), so a failed
/// update never clobbers a good cache.
pub(crate) fn update(feed: &dyn PriceFeed, as_of: &str, cache: &Path) -> Result<Updated, String> {
    let import = fetch_table(feed, as_of).map_err(|e: UpdateError| one_line(&e.to_string()))?;
    write_atomically(cache, import.table.to_toml().as_bytes())
        .map_err(|e| format!("cannot write price cache {}: {}", cache.display(), e.kind()))?;
    Ok(Updated {
        as_of: import.table.as_of().to_owned(),
        models: import.table.models().len(),
        skipped: import.skipped,
    })
}

fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp_name = path.file_name().map(OsString::from).unwrap_or_default();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = dir.join(tmp_name);
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// HTTP implementation of the core's [`PriceFeed`] seam.
struct HttpFeed {
    url: String,
}

impl PriceFeed for HttpFeed {
    fn fetch(&self) -> Result<String, FeedError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        let unavailable = |why: String| FeedError::Unavailable(why);
        let mut response = agent
            .get(&self.url)
            .header(
                "User-Agent",
                concat!("branch-tally/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|e| unavailable(e.to_string()))?;
        let mut text = String::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_FEED_BYTES)
            .read_to_string(&mut text)
            .map_err(|e| unavailable(format!("could not read the response: {}", e.kind())))?;
        Ok(text)
    }
}

/// Today's date in UTC as `YYYY-MM-DD`, from `now`.
pub(crate) fn today_utc(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day), after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Fake(Result<String, FeedError>);
    impl PriceFeed for Fake {
        fn fetch(&self) -> Result<String, FeedError> {
            self.0.clone()
        }
    }

    const GOOD: &str = r#"{"claude-a-1": {"litellm_provider": "anthropic", "mode": "chat",
        "input_cost_per_token": 1e-6, "output_cost_per_token": 5e-6}}"#;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn dates_convert_from_the_epoch() {
        assert_eq!(today_utc(at(0)), "1970-01-01");
        assert_eq!(today_utc(at(951_782_400)), "2000-02-29");
        assert_eq!(today_utc(at(1_078_099_200 - 1)), "2004-02-29");
        assert_eq!(today_utc(at(1_798_761_600 - 1)), "2026-12-31");
        assert_eq!(today_utc(at(1_798_761_600)), "2027-01-01");
    }

    #[test]
    fn cache_dir_follows_the_platform_rules() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let tail = Path::new("branch-tally").join("prices.toml");
        if cfg!(windows) {
            let got = cache_path_from(env(&[("LOCALAPPDATA", "C:\\L"), ("HOME", "C:\\H")]));
            assert_eq!(got, Some(Path::new("C:\\L").join(&tail)));
            assert_eq!(cache_path_from(env(&[("HOME", "C:\\H")])), None);
        } else if cfg!(target_os = "macos") {
            let got = cache_path_from(env(&[("HOME", "/h"), ("XDG_CACHE_HOME", "/x")]));
            assert_eq!(got, Some(Path::new("/h/Library/Caches").join(&tail)));
        } else {
            let got = cache_path_from(env(&[("HOME", "/h"), ("XDG_CACHE_HOME", "/x")]));
            assert_eq!(got, Some(Path::new("/x").join(&tail)));
            let got = cache_path_from(env(&[("HOME", "/h"), ("XDG_CACHE_HOME", "")]));
            assert_eq!(got, Some(Path::new("/h/.cache").join(&tail)));
            assert_eq!(cache_path_from(env(&[])), None);
        }
    }

    #[test]
    fn a_good_feed_writes_the_cache_with_lf_endings() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("deep").join("prices.toml");
        let done = update(&Fake(Ok(GOOD.into())), "2030-04-05", &cache).unwrap();
        assert_eq!(done.as_of, "2030-04-05");
        assert_eq!(done.models, 1);
        let text = std::fs::read_to_string(&cache).unwrap();
        assert!(text.starts_with("as_of = \"2030-04-05\""));
        assert!(!text.contains('\r'));
        let leftovers: Vec<_> = std::fs::read_dir(cache.parent().unwrap())
            .unwrap()
            .collect();
        assert_eq!(leftovers.len(), 1, "temp file left behind");
    }

    #[test]
    fn a_failed_fetch_or_conversion_leaves_the_cache_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("prices.toml");
        std::fs::write(&cache, "keep").unwrap();
        let down = Fake(Err(FeedError::Unavailable("down".into())));
        assert!(update(&down, "2030-04-05", &cache).is_err());
        assert!(update(&Fake(Ok("[]".into())), "2030-04-05", &cache).is_err());
        assert!(update(&Fake(Ok(GOOD.into())), "not-a-date", &cache).is_err());
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), "keep");
    }
}
