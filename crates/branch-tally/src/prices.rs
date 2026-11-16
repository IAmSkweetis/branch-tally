//! The `prices` subcommand and the price-table lookup `report` shares with it.
//!
//! Lookup order, first found wins whole: `.tally/prices.toml` in the main checkout (override),
//! the per-user cache written by `prices update`, then the table built into the binary.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use branch_tally_core::atomic::{self, OsSuffix, SuffixSource};
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

/// Replaces [`FEED_URL`], for a mirror or a proxy. The value must be an `https://` URL: anything
/// else is refused before any connection (#98), and the download agent is `https_only`, so a
/// redirect to plain http fails too. There is no way to trust another CA from here: tests that
/// need a local server run it over TLS in this module's unit tests.
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
            let url = match std::env::var(FEED_URL_ENV) {
                Ok(url) => check_feed_url(&url, Some(FEED_URL_ENV))?,
                Err(_) => FEED_URL.to_owned(),
            };
            let as_of = today_utc(SystemTime::now());
            let outcome = update(&HttpFeed::new(url), &as_of, &cache)?;
            Ok(outcome.describe(&cache))
        }
        PricesCommand::Show(args) => show(args),
    }
}

/// Accepts `url` only when it starts with `https://` (case-insensitive scheme). `var` names the
/// environment variable it came from. The value is never echoed: it is user-controlled text.
fn check_feed_url(url: &str, var: Option<&str>) -> Result<String, String> {
    let https = url
        .get(.."https://".len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if https {
        return Ok(url.to_owned());
    }
    let source = var.map_or_else(|| "the price feed URL".to_owned(), str::to_owned);
    Err(format!(
        "{source} must be an https:// URL; refusing to fetch prices over any other scheme"
    ))
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

/// Feed-derived text with control and bidi characters made inert before it reaches a terminal.
fn neutral(text: &str) -> std::borrow::Cow<'_, str> {
    branch_tally_core::text::replace_control_chars(text)
}

impl Updated {
    fn describe(&self, path: &Path) -> String {
        let mut out = format!(
            "Wrote {}\nas of: {}\nmodels: {}\n",
            path.display(),
            neutral(&self.as_of),
            self.models
        );
        if self.skipped.is_empty() {
            out.push_str("skipped: none\n");
        } else {
            let _ = writeln!(
                out,
                "skipped {} model(s) without a usable price: {}",
                self.skipped.len(),
                self.skipped
                    .iter()
                    .map(|id| neutral(id))
                    .collect::<Vec<_>>()
                    .join(", ")
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
    write_atomically_with(path, bytes, &mut OsSuffix)
}

/// [`write_atomically`] with the temp-name source injected, so a test can plant a file or symlink
/// at the exact name the writer would pick.
fn write_atomically_with(
    path: &Path,
    bytes: &[u8],
    suffixes: &mut dyn SuffixSource,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    atomic::write_atomic_with(path, bytes, suffixes)
}

/// HTTP implementation of the core's [`PriceFeed`] seam.
struct HttpFeed {
    url: String,
    agent: ureq::Agent,
}

impl HttpFeed {
    fn new(url: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .https_only(true)
            .build()
            .into();
        Self { url, agent }
    }

    /// Test-only: an agent that trusts the loopback fixture CA, still https-only.
    #[cfg(test)]
    fn with_fixture_ca(url: String) -> Self {
        Self {
            url,
            agent: crate::tls_test::fixture_agent(),
        }
    }
}

impl PriceFeed for HttpFeed {
    fn fetch(&self) -> Result<String, FeedError> {
        let unavailable = |why: String| FeedError::Unavailable(why);
        let mut response = self
            .agent
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

    /// Yields fixed suffixes and counts how many were asked for.
    struct Fixed {
        names: Vec<&'static str>,
        asked: usize,
    }
    impl SuffixSource for Fixed {
        fn next_suffix(&mut self) -> io::Result<String> {
            let next = self.names.get(self.asked).copied();
            self.asked += 1;
            next.map(str::to_owned)
                .ok_or_else(|| io::Error::other("exhausted"))
        }
    }

    fn fixed() -> Fixed {
        Fixed {
            names: vec!["aaaa", "bbbb"],
            asked: 0,
        }
    }

    /// The temp name the old writer used, which an attacker could predict.
    fn old_name(dir: &Path) -> PathBuf {
        dir.join(format!("prices.toml.tmp-{}", std::process::id()))
    }

    #[test]
    fn a_planted_file_at_the_temp_name_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let cache = sub.join("prices.toml");
        let planted = sub.join(".prices.toml.aaaa.tmp");
        std::fs::write(&planted, "victim").unwrap();
        std::fs::write(old_name(&sub), "victim-old").unwrap();
        let mut suffixes = fixed();
        write_atomically_with(&cache, b"new", &mut suffixes).unwrap();
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&planted).unwrap(), "victim");
        assert_eq!(
            std::fs::read_to_string(old_name(&sub)).unwrap(),
            "victim-old"
        );
        assert_eq!(suffixes.asked, 2, "the collision must pick a fresh name");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_at_the_temp_name_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "victim").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let cache = sub.join("prices.toml");
        std::os::unix::fs::symlink(&victim, sub.join(".prices.toml.aaaa.tmp")).unwrap();
        std::os::unix::fs::symlink(&victim, old_name(&sub)).unwrap();
        let mut suffixes = fixed();
        write_atomically_with(&cache, b"new", &mut suffixes).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), "new");
        assert_eq!(suffixes.asked, 2);
    }

    #[test]
    fn the_cache_directory_is_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("a").join("b").join("prices.toml");
        write_atomically(&cache, b"x").unwrap();
        assert_eq!(std::fs::read_to_string(&cache).unwrap(), "x");
    }

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

    #[test]
    fn feed_derived_text_is_neutralised_in_the_description() {
        let feed = r#"{"claude-a-1": {"litellm_provider": "anthropic", "mode": "chat",
            "input_cost_per_token": 1e-6, "output_cost_per_token": 5e-6},
            "claude-\u001b[2Jbad\u202e-2": {"litellm_provider": "anthropic", "mode": "chat"}}"#;
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("prices.toml");
        let done = update(&Fake(Ok(feed.into())), "2030-04-05", &cache).unwrap();
        assert!(!done.skipped.is_empty(), "the feed must skip the bad model");
        let text = done.describe(&cache);
        assert!(text.contains("bad"), "{text:?}");
        assert!(
            !text
                .chars()
                .filter(|c| *c != '\n')
                .any(branch_tally_core::text::is_unsafe_control),
            "{text:?}"
        );
    }

    // --- the feed over loopback TLS (these replace the old plain-http e2e tests, #98) ---

    use crate::tls_test::serve_tls_once;

    const FEED: &str = r#"{
      "sample_spec": {"litellm_provider": "x"},
      "claude-test-1": {"litellm_provider": "anthropic", "mode": "chat",
        "input_cost_per_token": 3e-6, "output_cost_per_token": 15e-6,
        "cache_read_input_token_cost": 3e-7, "cache_creation_input_token_cost": 3.75e-6},
      "claude-broken-1": {"litellm_provider": "anthropic", "mode": "chat"}
    }"#;

    #[test]
    fn a_tls_feed_writes_the_dated_cache_and_reports_what_it_skipped() {
        let server = serve_tls_once("200 OK", "Content-Type: application/json\r\n", FEED);
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("branch-tally").join("prices.toml");
        let done = update(
            &HttpFeed::with_fixture_ca(server.url.clone()),
            "2030-04-05",
            &cache,
        )
        .unwrap();
        assert!(server.finish());
        let text = std::fs::read_to_string(&cache).unwrap();
        assert!(!text.contains('\r'));
        assert!(text.contains("[models.\"claude-test-1\"]"), "{text}");
        assert!(!text.contains("claude-broken-1"));
        let said = done.describe(&cache);
        assert!(said.contains(&cache.display().to_string()), "{said}");
        assert!(said.contains("models: 1\n"), "{said}");
        assert!(said.contains("skipped"), "{said}");
        assert!(said.contains("claude-broken-1"), "{said}");
    }

    #[test]
    fn a_failing_tls_feed_leaves_an_existing_cache_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("prices.toml");
        std::fs::write(&cache, "keep").unwrap();
        for (status, body) in [
            ("500 Internal Server Error", "boom"),
            ("200 OK", "not json"),
        ] {
            let server = serve_tls_once(status, "", body);
            let err = update(
                &HttpFeed::with_fixture_ca(server.url.clone()),
                "2030-04-05",
                &cache,
            )
            .unwrap_err();
            assert!(server.finish());
            assert!(!err.contains("boom"), "{err}");
            assert_eq!(std::fs::read_to_string(&cache).unwrap(), "keep", "{status}");
        }
    }

    #[test]
    fn a_redirect_from_the_tls_feed_to_plain_http_is_refused() {
        let bait = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        bait.set_nonblocking(true).unwrap();
        let target = format!("http://{}/prices.json", bait.local_addr().unwrap());
        let server = serve_tls_once("302 Found", &format!("Location: {target}\r\n"), "");
        let err = HttpFeed::with_fixture_ca(server.url.clone())
            .fetch()
            .unwrap_err();
        assert!(matches!(err, FeedError::Unavailable(_)), "{err:?}");
        assert!(server.finish());
        assert!(bait.accept().is_err(), "the http target was contacted");
    }

    #[test]
    fn the_shipped_feed_agent_does_not_trust_the_fixture_ca() {
        let server = serve_tls_once("200 OK", "", "{}");
        let err = HttpFeed::new(server.url.clone()).fetch().unwrap_err();
        let _ = server.finish();
        let FeedError::Unavailable(why) = err;
        let why = why.to_lowercase();
        assert!(
            why.contains("certificate") || why.contains("unknownissuer") || why.contains("tls"),
            "not a trust failure: {why}"
        );
    }

    #[test]
    fn the_shipped_feed_agent_never_contacts_a_plain_http_url() {
        // `check_feed_url` is bypassed on purpose: this pins the agent's own `https_only`.
        let bait = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        bait.set_nonblocking(true).unwrap();
        let url = format!("http://{}/prices.json", bait.local_addr().unwrap());
        let err = HttpFeed::new(url).fetch().unwrap_err();
        assert!(matches!(err, FeedError::Unavailable(_)), "{err:?}");
        assert!(bait.accept().is_err(), "the plain http URL was contacted");
    }

    #[test]
    fn only_https_urls_pass_the_scheme_check() {
        assert!(check_feed_url("https://mirror.example/p.json", None).is_ok());
        assert!(check_feed_url("HTTPS://mirror.example/p.json", None).is_ok());
        for bad in [
            "http://m/p.json",
            "ftp://m",
            "file:///x",
            "m/p.json",
            "",
            "htp://m",
        ] {
            let err = check_feed_url(bad, Some(FEED_URL_ENV)).unwrap_err();
            assert!(err.contains(FEED_URL_ENV), "{err}");
            assert!(err.contains("https://"), "{err}");
            assert!(!err.contains(bad) || bad.is_empty(), "{err}");
        }
        assert!(
            !check_feed_url("\u{1b}[2J", Some("X"))
                .unwrap_err()
                .contains('\u{1b}')
        );
    }
}
