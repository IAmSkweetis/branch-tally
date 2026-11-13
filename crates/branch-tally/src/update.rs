//! The `update` subcommand: the real release client, installer and confirmer behind core's traits.
//!
//! Core decides what to do ([`branch_tally_core::update::run_verified`]); this file only talks to the
//! network, the filesystem and the terminal.

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use branch_tally_core::update::{
    Confirmer, FetchError, Installer, ReleaseClient, ReleaseKey, UpdateOptions, Version,
    current_target, is_binary_entry, run_verified,
};

use crate::cli::{UpdateArgs, VERSION};

/// Release mirror, same variable and default as `install.sh` and `install.ps1`. Also lets the
/// tests point the client at a `file://` fake release.
const BASE_URL_ENV: &str = "BRANCH_TALLY_INSTALL_BASE_URL";
const DEFAULT_BASE_URL: &str = "https://github.com/IAmSkweetis/branch-tally/releases";

/// The maintainer's release public key (id D9F252276FF0E4D0): the second line of
/// `.github/minisign.pub`, which the release job signs with. A test fails if the two differ.
/// There is deliberately no runtime way to replace it: no variable, flag or config.
const RELEASE_PUBLIC_KEY: &str = "RWTQ5PBvJ1Ly2e0dZqwbC5mduW++vTIp1vIHDe+C8z7/4VNNvoU30mmr";

/// Where `update` learns the latest tag. A constant: no variable, flag or config changes it.
const LATEST_API_URL: &str =
    "https://api.github.com/repos/IAmSkweetis/branch-tally/releases/latest";

/// The token for the GitHub API: `GH_TOKEN`, else `GITHUB_TOKEN`, ignoring empty values.
fn token_from(gh: Option<String>, github: Option<String>) -> Option<String> {
    [gh, github]
        .into_iter()
        .flatten()
        .map(|t| t.trim().to_owned())
        .find(|t| !t.is_empty())
}

/// The largest API response accepted. The latest-release JSON is a few KB.
const MAX_API_BYTES: u64 = 1024 * 1024;

/// The largest download accepted. Release archives are a few MB; this only stops a runaway body.
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// The binary's file name inside the archive and on disk.
const BIN_NAME: &str = if cfg!(windows) {
    "branch-tally.exe"
} else {
    "branch-tally"
};

/// Run `update`. `Ok` is the line for stdout; `Err` is the line for stderr (exit 1).
pub(crate) fn run(args: &UpdateArgs) -> Result<String, String> {
    let base = std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let base = check_base_url(&base)?;
    let options = UpdateOptions {
        check: args.check,
        version: args.version.clone(),
        yes: args.yes,
        force: args.force,
    };
    let target = current_target();
    let key = ReleaseKey::from_base64(RELEASE_PUBLIC_KEY).map_err(|e| e.to_string())?;
    let outcome = run_verified(
        VERSION,
        target,
        &options,
        &key,
        &HttpClient::new(&base),
        &FsInstaller {
            exe: current_exe(),
            target,
        },
        &TerminalConfirmer,
    )
    .map_err(|e| e.to_string())?;
    if outcome.is_refusal() {
        Err(outcome.message())
    } else {
        Ok(outcome.message())
    }
}

/// Accept only `https://` and `file://` mirrors, before any network call.
fn check_base_url(base: &str) -> Result<String, String> {
    let base = base.trim();
    if base.starts_with("https://") || base.starts_with("file://") {
        Ok(base.to_owned())
    } else {
        Err(format!(
            "{BASE_URL_ENV} must start with https:// or file:// (plain http is not accepted)"
        ))
    }
}

/// The running executable with symlinks resolved, so a symlinked install updates its target.
fn current_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(BIN_NAME));
    std::fs::canonicalize(&exe).unwrap_or(exe)
}

// ---------------------------------------------------------------------------------------------
// Release client

/// Reads a release mirror over HTTPS, or from a directory for a `file://` base.
struct HttpClient {
    base: String,
    api_url: String,
    token: Option<String>,
    https_only: bool,
}

impl HttpClient {
    /// The shipped client: the API URL is the constant, the token comes from the environment.
    fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            api_url: LATEST_API_URL.to_owned(),
            token: token_from(
                std::env::var("GH_TOKEN").ok(),
                std::env::var("GITHUB_TOKEN").ok(),
            ),
            https_only: true,
        }
    }

    /// A client with a test API URL and token. Not compiled into the shipped binary.
    #[cfg(test)]
    fn with_api(base: &str, api_url: &str, token: Option<&str>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            api_url: api_url.to_owned(),
            token: token.map(str::to_owned),
            https_only: false,
        }
    }

    /// The API's JSON for the latest release, capped like the downloads.
    fn fetch_api(&self) -> Result<Vec<u8>, FetchError> {
        let fail = |reason: String| FetchError::Failed(format!("GitHub API: {reason}"));
        if let Some(path) = self.api_url.strip_prefix("file://") {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
            return read_local(dir, name, MAX_API_BYTES).map_err(|e| match e {
                FetchError::Failed(reason) => fail(reason),
                FetchError::NotFound => fail("not found".to_owned()),
            });
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .https_only(self.https_only)
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent
            .get(&self.api_url)
            .header(
                "User-Agent",
                concat!("branch-tally/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        // Error text is built from the status or a fixed phrase only, never from the request.
        let mut response = request
            .call()
            .map_err(|_| fail("could not be reached".to_owned()))?;
        let status = response.status().as_u16();
        if status != 200 {
            let hint = if matches!(status, 403 | 429) {
                " (possibly rate limited)"
            } else {
                ""
            };
            return Err(fail(format!("HTTP status {status}{hint}")));
        }
        read_capped(response.body_mut().as_reader(), MAX_API_BYTES).map_err(fail)
    }

    fn fetch(&self, path: &str) -> Result<Vec<u8>, FetchError> {
        match self.base.strip_prefix("file://") {
            Some(dir) => read_local(dir, path, MAX_DOWNLOAD_BYTES),
            None => fetch_http(&format!("{}/{path}", self.base), MAX_DOWNLOAD_BYTES),
        }
    }
}

impl ReleaseClient for HttpClient {
    fn latest_tag(&self) -> Result<String, FetchError> {
        // The mirror is never consulted here: a mirror could serve an older signed release as
        // "latest" and roll users back. Any API problem is a failure, not a fallback.
        let body = self.fetch_api()?;
        latest_tag_from_api(&body).map_err(FetchError::Failed)
    }

    fn download(&self, tag: &str, asset: &str) -> Result<Vec<u8>, FetchError> {
        self.fetch(&format!("download/{tag}/{asset}"))
    }
}

/// The `tag_name` of a latest-release response, validated as `vMAJOR.MINOR.PATCH`.
fn latest_tag_from_api(body: &[u8]) -> Result<String, String> {
    let tag = top_level_string(body, "tag_name")?;
    match Version::parse(&tag) {
        Ok(version) if tag.starts_with('v') && version.tag() == tag => Ok(tag),
        _ => Err("GitHub API: tag_name was not vMAJOR.MINOR.PATCH".to_owned()),
    }
}

/// The string value of a top-level `key` in a JSON object, and nothing else.
///
/// The binary does not depend on a JSON crate, so this is a strict scanner for the one shape it
/// needs. Nested values are skipped with string-awareness, so a `tag_name` inside `assets` or
/// `author` is never taken. Anything malformed, a duplicate key, a non-string value, or a value
/// with an escape sequence is an error: a tag never needs one.
fn top_level_string(body: &[u8], key: &str) -> Result<String, String> {
    let bad = || "GitHub API: the response was not the expected JSON".to_owned();
    let mut at = Scanner { bytes: body, at: 0 };
    at.space();
    at.expect(b'{').ok_or_else(bad)?;
    let mut found: Option<String> = None;
    at.space();
    if at.peek() == Some(b'}') {
        return Err("GitHub API: the response had no tag_name".to_owned());
    }
    loop {
        at.space();
        let name = at.string().ok_or_else(bad)?;
        at.space();
        at.expect(b':').ok_or_else(bad)?;
        at.space();
        if name == key.as_bytes() {
            if found.is_some() {
                return Err(bad());
            }
            let value = at.string().ok_or("GitHub API: tag_name was not a string")?;
            if value.contains(&b'\\') {
                return Err(bad());
            }
            found = Some(String::from_utf8(value).map_err(|_| bad())?);
        } else {
            at.skip_value(0).ok_or_else(bad)?;
        }
        at.space();
        match at.next() {
            Some(b',') => {}
            Some(b'}') => break,
            _ => return Err(bad()),
        }
    }
    at.space();
    if at.peek().is_some() {
        return Err(bad());
    }
    found.ok_or_else(|| "GitHub API: the response had no tag_name".to_owned())
}

struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.at += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.next()? == byte).then_some(())
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    /// A string's raw bytes (escapes left as written), consuming both quotes.
    fn string(&mut self) -> Option<Vec<u8>> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.next()? {
                b'"' => return Some(out),
                b'\\' => {
                    out.push(b'\\');
                    out.push(self.next()?);
                }
                byte if byte < 0x20 => return None,
                byte => out.push(byte),
            }
        }
    }

    /// Skip one value of any kind. Depth is capped so hostile nesting cannot overflow the stack.
    fn skip_value(&mut self, depth: usize) -> Option<()> {
        if depth > 64 {
            return None;
        }
        match self.peek()? {
            b'"' => self.string().map(|_| ()),
            open @ (b'{' | b'[') => {
                self.at += 1;
                let close = if open == b'{' { b'}' } else { b']' };
                self.space();
                if self.peek()? == close {
                    self.at += 1;
                    return Some(());
                }
                loop {
                    self.space();
                    if open == b'{' {
                        self.string()?;
                        self.space();
                        self.expect(b':')?;
                        self.space();
                    }
                    self.skip_value(depth + 1)?;
                    self.space();
                    match self.next()? {
                        b',' => {}
                        c if c == close => return Some(()),
                        _ => return None,
                    }
                }
            }
            _ => {
                let start = self.at;
                while matches!(self.peek(), Some(b) if !matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n'))
                {
                    self.at += 1;
                }
                (self.at > start).then_some(())
            }
        }
    }
}

/// Read `path` (slash-separated, relative) under a `file://` directory.
fn read_local(dir: &str, path: &str, cap: u64) -> Result<Vec<u8>, FetchError> {
    // `file:///C:/x` carries a leading slash before the drive letter.
    let bytes = dir.as_bytes();
    let dir = if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' && cfg!(windows) {
        &dir[1..]
    } else {
        dir
    };
    let file = path
        .split('/')
        .fold(PathBuf::from(dir), |acc, part| acc.join(part));
    let handle = std::fs::File::open(file).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => FetchError::NotFound,
        kind => FetchError::Failed(format!("could not read the release files: {kind}")),
    })?;
    read_capped(handle, cap).map_err(FetchError::Failed)
}

fn fetch_http(url: &str, cap: u64) -> Result<Vec<u8>, FetchError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("branch-tally/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => FetchError::NotFound,
            ureq::Error::StatusCode(code) => FetchError::Failed(format!("HTTP status {code}")),
            other => FetchError::Failed(other.to_string()),
        })?;
    read_capped(response.body_mut().as_reader(), cap).map_err(FetchError::Failed)
}

// ---------------------------------------------------------------------------------------------
// Installer

/// Replaces the executable at `exe`.
struct FsInstaller {
    exe: PathBuf,
    target: &'static str,
}

impl Installer for FsInstaller {
    fn check_writable(&self) -> Result<(), String> {
        check_dir_writable(&self.exe)
    }

    fn install(&self, asset: &str, archive: &[u8]) -> Result<(), String> {
        let bytes = extract_binary(asset, archive, self.target, MAX_DOWNLOAD_BYTES)?;
        replace_binary(&self.exe, &bytes, cfg!(windows))
    }
}

/// Create and remove a probe file in the directory holding `exe`.
fn check_dir_writable(exe: &Path) -> Result<(), String> {
    let dir = exe.parent().ok_or("the binary has no parent directory")?;
    let probe = dir.join(format!(".branch-tally-write-test-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| format!("{}", e.kind()))?;
    std::fs::remove_file(&probe).map_err(|e| format!("{}", e.kind()))
}

/// Pull the binary out of the archive by file name only. No archive path is ever joined onto a
/// filesystem path, and entries with `..` or absolute components are skipped.
fn extract_binary(asset: &str, archive: &[u8], target: &str, cap: u64) -> Result<Vec<u8>, String> {
    let found = if asset.ends_with(".tar.gz") {
        extract_from_tar_gz(archive, target, cap)?
    } else if Path::new(asset)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        extract_from_zip(archive, target, cap)?
    } else {
        return Err("unknown archive type".to_owned());
    };
    found.ok_or_else(|| format!("the archive does not contain {}", binary_name(target)))
}

fn binary_name(target: &str) -> &'static str {
    if target.contains("-windows-") {
        "branch-tally.exe"
    } else {
        "branch-tally"
    }
}

/// Read all of `reader`, failing when it holds more than `cap` bytes rather than truncating.
fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read the download: {}", e.kind()))?;
    if bytes.len() as u64 > cap {
        return Err(format!("the download is larger than the {cap}-byte limit"));
    }
    Ok(bytes)
}

fn extract_from_tar_gz(archive: &[u8], target: &str, cap: u64) -> Result<Option<Vec<u8>>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let unreadable = |e: io::Error| format!("could not read the archive: {}", e.kind());
    for entry in tar.entries().map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if is_binary_entry(&path, target) {
            return read_capped(entry, cap).map(Some);
        }
    }
    Ok(None)
}

fn extract_from_zip(archive: &[u8], target: &str, cap: u64) -> Result<Option<Vec<u8>>, String> {
    let unreadable = |e: zip::result::ZipError| format!("could not read the archive: {e}");
    let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(unreadable)?;
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(unreadable)?;
        if file.is_file() && file.name().is_ok_and(|name| is_binary_entry(&name, target)) {
            return read_capped(file, cap).map(Some);
        }
    }
    Ok(None)
}

/// Write `bytes` beside `exe` and move it into place so the old binary is never half-replaced.
fn replace_binary(exe: &Path, bytes: &[u8], windows: bool) -> Result<(), String> {
    replace_binary_with(exe, bytes, windows, &|from, to| std::fs::rename(from, to))
}

type Rename<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

fn replace_binary_with(
    exe: &Path,
    bytes: &[u8],
    windows: bool,
    rename: Rename<'_>,
) -> Result<(), String> {
    let dir = exe.parent().ok_or("the binary has no parent directory")?;
    let staged = stage_new_binary(dir, exe, bytes)?;
    let result = if windows {
        swap_aside(exe, &staged, rename)
    } else {
        rename(&staged, exe).map_err(|e| fail("could not replace the binary", &e))
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn fail(what: &str, e: &io::Error) -> String {
    format!("{what}: {}", e.kind())
}

/// Create the staged file exclusively (never through a planted symlink), fill it, and flush it to
/// disk. A failure removes whatever was created.
fn stage_new_binary(dir: &Path, exe: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let mut attempt = 0u32;
    let (staged, mut file) = loop {
        let candidate = dir.join(format!(
            ".branch-tally-new-{}{}",
            std::process::id(),
            if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            }
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 8 => attempt += 1,
            Err(e) => return Err(fail("could not create the new binary", &e)),
        }
    };
    let written = (|| {
        file.write_all(bytes)
            .map_err(|e| fail("could not write the new binary", &e))?;
        match_permissions(&file, exe)
            .map_err(|e| fail("could not mark the new binary executable", &e))?;
        file.sync_all()
            .map_err(|e| fail("could not flush the new binary to disk", &e))
    })();
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    Ok(staged)
}

/// Give the staged file the old binary's permission bits, or `0o755` when those cannot be read.
#[cfg(unix)]
fn match_permissions(file: &std::fs::File, exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(exe).map_or(0o755, |m| m.permissions().mode() & 0o7777);
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

/// Windows has no mode bits to carry over.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "keeps the signature of the Unix version, which can fail"
)]
fn match_permissions(_file: &std::fs::File, _exe: &Path) -> io::Result<()> {
    Ok(())
}

/// Windows cannot overwrite a running exe but can rename it: move it to `.old`, then the new one
/// into place, restoring `.old` if that fails.
fn swap_aside(exe: &Path, staged: &Path, rename: Rename<'_>) -> Result<(), String> {
    let old = old_path(exe);
    if old.exists()
        && let Err(e) = std::fs::remove_file(&old)
    {
        return Err(format!(
            "could not remove the stale {} left by a previous update (is it still running?): {}",
            old.display(),
            e.kind()
        ));
    }
    rename(exe, &old).map_err(|e| fail("could not move the old binary aside", &e))?;
    let Err(e) = rename(staged, exe) else {
        return Ok(());
    };
    match rename(&old, exe) {
        Ok(()) => Err(fail("could not move the new binary into place", &e)),
        Err(restore) => Err(format!(
            "could not move the new binary into place ({}), and restoring the old one failed ({}). \
             The old binary was moved to {}; rename it back to {} to restore it.",
            e.kind(),
            restore.kind(),
            old.display(),
            exe.display()
        )),
    }
}

/// `branch-tally.exe` becomes `branch-tally.exe.old`.
fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------------------------
// Confirmer

struct TerminalConfirmer;

impl Confirmer for TerminalConfirmer {
    fn confirm(&self, prompt: &str) -> bool {
        let mut stderr = io::stderr();
        if !io::stdin().is_terminal() {
            let _ = writeln!(
                stderr,
                "{prompt} Not a terminal, so no confirmation was possible; pass --yes to proceed."
            );
            return false;
        }
        let _ = write!(stderr, "{prompt} [y/N] ");
        let _ = stderr.flush();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).is_err() {
            return false;
        }
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use branch_tally_core::update::{Outcome, UpdateError};
    use branch_tally_core::update::{SIGNATURE_NAME, SUMS_NAME, sha256_hex};

    const TARGET: &str = "x86_64-unknown-linux-musl";
    /// Tests always model a Linux archive, so entry and file names do not depend on the host.
    const BIN_NAME: &str = "branch-tally";
    const CAP: u64 = MAX_DOWNLOAD_BYTES;
    const GOOD: &[u8] = b"new binary bytes";

    // --- fixtures built in the test ---

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            // Written raw so a `..` name reaches the archive; `set_path` would refuse it.
            let raw = header.as_old_mut();
            raw.name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// Throwaway test key and signatures from core's test data. They sign the sums for the bytes
    /// [`SIGNED_ARCHIVE`], so a release built from them is installable only with this key.
    const TEST_KEY: &str =
        include_str!("../../branch-tally-core/tests/data/update_signing/test-public-key.txt");
    const GOOD_SIG: &[u8] =
        include_bytes!("../../branch-tally-core/tests/data/update_signing/good-v1.3.0.minisig");
    const OTHER_KEY_SIG: &[u8] = include_bytes!(
        "../../branch-tally-core/tests/data/update_signing/other-key-v1.3.0.minisig"
    );
    const SIGNED_ARCHIVE: &[u8] = b"fake archive bytes";
    const SIGNED_ASSET: &str = "branch-tally-v1.3.0-x86_64-unknown-linux-musl.tar.gz";

    /// A fake release directory holding `v1.3.0` for [`TARGET`], plus a `latest` alias of it, with
    /// `signature` as `SHA256SUMS.minisig` when given.
    fn fake_release(dir: &Path, archive: &[u8], signature: Option<&[u8]>) -> String {
        let sums = format!("{}  {SIGNED_ASSET}\n", sha256_hex(SIGNED_ARCHIVE));
        for sub in ["download/v1.3.0", "latest/download"] {
            let d = dir.join(sub);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(SUMS_NAME), &sums).unwrap();
            std::fs::write(d.join(SIGNED_ASSET), archive).unwrap();
            if let Some(signature) = signature {
                std::fs::write(d.join(SIGNATURE_NAME), signature).unwrap();
            }
        }
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::write(dir.join("api/latest.json"), r#"{"tag_name":"v1.3.0"}"#).unwrap();
        format!("file://{}", dir.to_string_lossy().replace('\\', "/"))
    }

    struct Yes;
    impl Confirmer for Yes {
        fn confirm(&self, _: &str) -> bool {
            true
        }
    }

    fn release_archive() -> Vec<u8> {
        tar_gz(&[
            ("branch-tally-v1.1.0/LICENSE", b"license"),
            (&format!("branch-tally-v1.1.0/{BIN_NAME}"), GOOD),
            ("branch-tally-v1.1.0/README.md", b"readme"),
        ])
    }

    // --- extraction ---

    #[test]
    fn extracts_only_the_binary_from_a_tar_gz() {
        let archive = release_archive();
        assert_eq!(
            extract_binary("branch-tally-v1.1.0-x.tar.gz", &archive, TARGET, CAP).unwrap(),
            GOOD
        );
    }

    #[test]
    fn extracts_only_the_binary_from_a_zip() {
        let archive = zip_of(&[
            ("branch-tally-v1.1.0/LICENSE", b"license"),
            (&format!("branch-tally-v1.1.0/{BIN_NAME}"), GOOD),
        ]);
        assert_eq!(
            extract_binary("branch-tally-v1.1.0-x.zip", &archive, TARGET, CAP).unwrap(),
            GOOD
        );
    }

    #[test]
    fn a_traversal_entry_is_ignored_in_tar_and_zip() {
        let decoy: &[u8] = b"decoy";
        let entries: [(&str, &[u8]); 3] = [
            ("../../evil", decoy),
            (&format!("../{BIN_NAME}"), decoy),
            (&format!("dir/{BIN_NAME}"), GOOD),
        ];
        assert_eq!(
            extract_binary("a.tar.gz", &tar_gz(&entries), TARGET, CAP).unwrap(),
            GOOD
        );
        assert_eq!(
            extract_binary("a.zip", &zip_of(&entries), TARGET, CAP).unwrap(),
            GOOD
        );
    }

    #[test]
    fn an_archive_without_the_binary_is_an_error() {
        let archive = tar_gz(&[("dir/LICENSE", b"license")]);
        assert!(extract_binary("a.tar.gz", &archive, TARGET, CAP).is_err());
        assert!(extract_binary("a.tar.gz", b"not an archive", TARGET, CAP).is_err());
        assert!(extract_binary("a.rar", b"", TARGET, CAP).is_err());
    }

    // --- replacement ---

    #[test]
    fn replacing_leaves_a_working_file_at_the_target_and_no_stragglers() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        replace_binary(&exe, GOOD, false).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), GOOD);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != BIN_NAME && n != &format!("{BIN_NAME}.old"))
            .collect();
        assert!(names.is_empty(), "left behind: {names:?}");
    }

    #[test]
    fn a_failed_replace_keeps_the_old_binary() {
        let dir = tempfile::tempdir().unwrap();
        // The target is a non-empty directory: nothing can be renamed over it.
        let exe = dir.path().join(BIN_NAME);
        std::fs::create_dir(&exe).unwrap();
        std::fs::write(exe.join("keep"), b"x").unwrap();
        assert!(replace_binary(&exe, GOOD, false).is_err());
        assert!(exe.join("keep").exists());
    }

    #[test]
    fn a_missing_directory_is_not_writable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gone").join(BIN_NAME);
        assert!(check_dir_writable(&exe).is_err());
        assert!(check_dir_writable(&dir.path().join(BIN_NAME)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores directory modes, so only assert when the mode is really enforced.
        let enforced = std::fs::write(ro.join("probe"), b"").is_err();
        let result = check_dir_writable(&ro.join(BIN_NAME));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        if enforced {
            assert!(result.is_err());
        }
    }

    // --- the compiled-in release key ---

    #[test]
    fn the_compiled_in_key_is_the_one_committed_for_the_release_job() {
        let pub_file = include_str!("../../../.github/minisign.pub");
        let line2 = pub_file.lines().nth(1).unwrap().trim();
        assert_eq!(RELEASE_PUBLIC_KEY, line2);
    }

    #[test]
    fn the_compiled_in_key_decodes() {
        assert!(ReleaseKey::from_base64(RELEASE_PUBLIC_KEY).is_ok());
    }

    // --- client and verification, through core's run_verified against a file:// release ---

    /// A recording installer, so the happy path needs no real archive (extraction and replacement
    /// are covered above against the real installer).
    #[derive(Default)]
    struct Recorder {
        installed: std::cell::RefCell<Vec<(String, Vec<u8>)>>,
    }
    impl Installer for Recorder {
        fn check_writable(&self) -> Result<(), String> {
            Ok(())
        }
        fn install(&self, asset: &str, archive: &[u8]) -> Result<(), String> {
            self.installed
                .borrow_mut()
                .push((asset.to_owned(), archive.to_vec()));
            Ok(())
        }
    }

    fn verified(
        key: &str,
        base: &str,
        installer: &dyn Installer,
        options: &UpdateOptions,
    ) -> Result<Outcome, UpdateError> {
        run_verified(
            "1.0.0",
            TARGET,
            options,
            &ReleaseKey::from_base64(key.trim()).unwrap(),
            &HttpClient::with_api(base, &format!("{base}/api/latest.json"), None),
            installer,
            &Yes,
        )
    }

    fn yes() -> UpdateOptions {
        UpdateOptions {
            yes: true,
            ..UpdateOptions::default()
        }
    }

    #[test]
    fn a_signed_release_installs_through_the_file_client() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let rec = Recorder::default();
        let outcome = verified(TEST_KEY, &base, &rec, &yes()).unwrap();
        assert_eq!(outcome.message(), "updated branch-tally v1.0.0 -> v1.3.0");
        let installed = rec.installed.borrow();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].0, SIGNED_ASSET);
        assert_eq!(installed[0].1, SIGNED_ARCHIVE);
    }

    #[test]
    fn the_client_fetches_the_signature_through_download() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let got = HttpClient::with_api(&base, "file:///unused", None)
            .download("v1.3.0", SIGNATURE_NAME)
            .unwrap();
        assert_eq!(got, GOOD_SIG);
    }

    #[test]
    fn an_unsigned_release_is_refused_with_the_real_key_and_installs_nothing() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, None);
        let rec = Recorder::default();
        let err = verified(RELEASE_PUBLIC_KEY, &base, &rec, &yes()).unwrap_err();
        assert!(err.to_string().contains("is not signed"), "{err}");
        assert!(err.to_string().contains("SECURITY.md"), "{err}");
        assert!(rec.installed.borrow().is_empty());
    }

    #[test]
    fn a_signature_by_another_key_is_refused_with_the_real_key_and_installs_nothing() {
        let release = tempfile::tempdir().unwrap();
        // Signed by a throwaway key, checked against the real release key.
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let rec = Recorder::default();
        let err = verified(RELEASE_PUBLIC_KEY, &base, &rec, &yes()).unwrap_err();
        assert!(err.to_string().contains("different key"), "{err}");
        assert!(rec.installed.borrow().is_empty());
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(OTHER_KEY_SIG));
        let err = verified(RELEASE_PUBLIC_KEY, &base, &rec, &yes()).unwrap_err();
        assert!(err.to_string().contains("different key"), "{err}");
        assert!(rec.installed.borrow().is_empty());
    }

    #[test]
    fn a_garbage_signature_is_refused_and_installs_nothing() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(b"not a signature"));
        let rec = Recorder::default();
        let err = verified(TEST_KEY, &base, &rec, &yes()).unwrap_err();
        assert!(err.to_string().contains("malformed"), "{err}");
        assert!(rec.installed.borrow().is_empty());
    }

    #[test]
    fn check_reports_the_latest_without_touching_the_binary() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let rec = Recorder::default();
        let options = UpdateOptions {
            check: true,
            ..UpdateOptions::default()
        };
        let outcome = verified(TEST_KEY, &base, &rec, &options).unwrap();
        assert!(outcome.message().contains("v1.3.0"));
        assert!(rec.installed.borrow().is_empty());
    }

    #[test]
    fn an_unknown_tag_is_a_not_found_message() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let rec = Recorder::default();
        let options = UpdateOptions {
            yes: true,
            version: Some("v9.9.9".to_owned()),
            ..UpdateOptions::default()
        };
        let err = verified(TEST_KEY, &base, &rec, &options).unwrap_err();
        assert!(err.to_string().contains("release not found"), "{err}");
        assert!(rec.installed.borrow().is_empty());
    }

    #[test]
    fn a_tampered_archive_installs_nothing() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        std::fs::write(
            release.path().join("download/v1.3.0").join(SIGNED_ASSET),
            b"tampered",
        )
        .unwrap();
        let rec = Recorder::default();
        let err = verified(TEST_KEY, &base, &rec, &yes()).unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        assert!(rec.installed.borrow().is_empty());
    }

    // --- latest comes from the GitHub API ---

    /// An HTTP server answering every request with `status` and `body`, recording the raw
    /// request heads.
    struct FakeServer {
        url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeServer {
        fn start(status: u16, body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = requests.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    seen.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&head).into_owned());
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Self { url, requests }
        }

        fn heads(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn api_file(dir: &Path, json: &str) -> String {
        let file = dir.join("latest.json");
        std::fs::write(&file, json).unwrap();
        format!("file://{}", file.to_string_lossy().replace('\\', "/"))
    }

    fn failed(got: Result<String, FetchError>) -> String {
        let err = got.unwrap_err();
        assert!(matches!(err, FetchError::Failed(_)), "{err:?}");
        match err {
            FetchError::Failed(reason) => reason,
            FetchError::NotFound => String::new(),
        }
    }

    #[test]
    fn latest_is_taken_from_the_api_not_the_mirror() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        // The mirror's `latest` says v1.3.0; the API says v2.0.0.
        let api_dir = tempfile::tempdir().unwrap();
        let api = api_file(api_dir.path(), r#"{"tag_name":"v2.0.0","name":"x"}"#);
        let got = HttpClient::with_api(&base, &api, None).latest_tag();
        assert_eq!(got, Ok("v2.0.0".to_owned()));
    }

    #[test]
    fn the_token_is_sent_as_a_bearer_when_set_with_the_api_headers() {
        let server = FakeServer::start(200, r#"{"tag_name":"v1.4.0"}"#);
        let got =
            HttpClient::with_api("file:///unused", &server.url, Some("sekrit-token")).latest_tag();
        assert_eq!(got, Ok("v1.4.0".to_owned()));
        let head = server.heads().remove(0).to_ascii_lowercase();
        assert!(
            head.contains("authorization: bearer sekrit-token"),
            "{head}"
        );
        assert!(
            head.contains("accept: application/vnd.github+json"),
            "{head}"
        );
        assert!(head.contains("x-github-api-version: 2022-11-28"), "{head}");
        assert!(head.contains("user-agent: branch-tally/"), "{head}");
    }

    #[test]
    fn no_authorization_header_is_sent_without_a_token() {
        let server = FakeServer::start(200, r#"{"tag_name":"v1.4.0"}"#);
        HttpClient::with_api("file:///unused", &server.url, None)
            .latest_tag()
            .unwrap();
        let head = server.heads().remove(0).to_ascii_lowercase();
        assert!(!head.contains("authorization"), "{head}");
    }

    #[test]
    fn gh_token_wins_over_github_token_and_empty_values_are_ignored() {
        let s = |v: &str| Some(v.to_owned());
        assert_eq!(token_from(s("a"), s("b")), s("a"));
        assert_eq!(token_from(None, s("b")), s("b"));
        assert_eq!(token_from(s("  "), s("b")), s("b"));
        assert_eq!(token_from(None, None), None);
    }

    #[test]
    fn the_shipped_client_uses_the_https_api_constant() {
        let client = HttpClient::new("https://mirror.invalid");
        assert_eq!(client.api_url, LATEST_API_URL);
        assert!(client.https_only);
        assert!(
            LATEST_API_URL.starts_with("https://api.github.com/repos/IAmSkweetis/branch-tally/")
        );
    }

    #[test]
    fn an_api_failure_fails_without_touching_the_mirror_or_leaking_the_token() {
        let mirror = FakeServer::start(200, "ignored");
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let reason =
            failed(HttpClient::with_api(&mirror.url, &dead, Some("sekrit-token")).latest_tag());
        assert!(!reason.contains("sekrit-token"), "{reason}");
        assert_eq!(mirror.heads(), Vec::<String>::new());
    }

    #[test]
    fn a_non_200_status_fails_without_touching_the_mirror_or_leaking_the_token() {
        let mirror = FakeServer::start(200, "ignored");
        for status in [403u16, 404, 429, 500] {
            let api = FakeServer::start(status, r#"{"message":"rate limit exceeded"}"#);
            let reason = failed(
                HttpClient::with_api(&mirror.url, &api.url, Some("sekrit-token")).latest_tag(),
            );
            assert!(reason.contains(&status.to_string()), "{reason}");
            assert!(!reason.contains("sekrit-token"), "{reason}");
        }
        assert_eq!(mirror.heads(), Vec::<String>::new());
    }

    #[test]
    fn malformed_json_or_a_bad_tag_name_fails_without_touching_the_mirror() {
        let mirror = FakeServer::start(200, "ignored");
        let dir = tempfile::tempdir().unwrap();
        for body in [
            "not json",
            "{}",
            r#"{"tag_name":7}"#,
            r#"{"tag_name":"1.2.3"}"#,
            r#"{"tag_name":"v1.2"}"#,
            r#"{"tag_name":"v1.2.3-rc1"}"#,
            r#"{"tag_name":"v1.2.3/../x"}"#,
            r#"{"tag_name":""}"#,
        ] {
            let api = api_file(dir.path(), body);
            let got = HttpClient::with_api(&mirror.url, &api, Some("sekrit-token")).latest_tag();
            let reason = failed(got);
            assert!(!reason.contains("sekrit-token"), "{body}: {reason}");
        }
        assert_eq!(mirror.heads(), Vec::<String>::new());
    }

    #[test]
    fn only_the_top_level_tag_name_is_read_from_a_real_shaped_response() {
        let body = br#" {"url":"https://x/y","assets":[{"tag_name":"v9.9.9","n":[1,2.5,null,true]}],
            "author":{"tag_name":"v8.8.8"},"tag_name":"v1.2.3","body":"a \"q\" b\n","e":{},"l":[]} "#;
        assert_eq!(latest_tag_from_api(body), Ok("v1.2.3".to_owned()));
        for bad in [
            &br#"{"tag_name":"v1.0.\u0030"}"#[..],
            br#"{"tag_name":"v1.0.0","tag_name":"v2.0.0"}"#,
            br#"{"assets":[{"tag_name":"v1.0.0"}]}"#,
            br#"{"tag_name":"v1.0.0"} trailing"#,
            br#"{"tag_name":"v1.0.0""#,
            br#"[{"tag_name":"v1.0.0"}]"#,
        ] {
            assert!(latest_tag_from_api(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_oversized_api_response_fails() {
        let dir = tempfile::tempdir().unwrap();
        let big = format!(
            r#"{{"tag_name":"v1.0.0","body":"{}"}}"#,
            "a".repeat(2 * 1024 * 1024)
        );
        let api = api_file(dir.path(), &big);
        failed(HttpClient::with_api("file:///unused", &api, None).latest_tag());
    }

    #[test]
    fn a_pin_makes_no_api_call() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), SIGNED_ARCHIVE, Some(GOOD_SIG));
        let api = FakeServer::start(500, "");
        let client = HttpClient::with_api(&base, &api.url, None);
        let options = UpdateOptions {
            yes: true,
            version: Some("v1.3.0".to_owned()),
            ..UpdateOptions::default()
        };
        let rec = Recorder::default();
        run_verified(
            "1.0.0",
            TARGET,
            &options,
            &ReleaseKey::from_base64(TEST_KEY.trim()).unwrap(),
            &client,
            &rec,
            &Yes,
        )
        .unwrap();
        assert_eq!(api.heads(), Vec::<String>::new());
        assert_eq!(rec.installed.borrow().len(), 1);
    }

    #[test]
    fn a_missing_file_release_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let base = format!("file://{}", dir.path().to_string_lossy());
        let client = HttpClient::with_api(&base, "file:///unused", None);
        assert_eq!(client.download("v1.0.0", "x"), Err(FetchError::NotFound));
    }

    // --- download cap ---

    #[test]
    fn a_body_over_the_cap_is_an_error_not_a_truncation() {
        let err = read_capped(&[7u8; 11][..], 10).unwrap_err();
        assert!(err.contains("larger than"), "{err}");
        assert_eq!(read_capped(&[7u8; 10][..], 10).unwrap(), vec![7u8; 10]);
    }

    #[test]
    fn an_extracted_binary_over_the_cap_is_an_error_in_tar_and_zip() {
        let entries: [(&str, &[u8]); 1] = [("d/branch-tally", GOOD)];
        let cap = GOOD.len() as u64 - 1;
        let tar_err = extract_binary("a.tar.gz", &tar_gz(&entries), TARGET, cap).unwrap_err();
        let zip_err = extract_binary("a.zip", &zip_of(&entries), TARGET, cap).unwrap_err();
        assert!(tar_err.contains("larger than"), "{tar_err}");
        assert!(zip_err.contains("larger than"), "{zip_err}");
        let exact = GOOD.len() as u64;
        assert_eq!(
            extract_binary("a.zip", &zip_of(&entries), TARGET, exact).unwrap(),
            GOOD
        );
    }

    #[test]
    fn an_http_body_over_the_cap_is_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n01234567890123456789",
            );
        });
        let err = fetch_http(&url, 10).unwrap_err();
        server.join().unwrap();
        assert!(format!("{err:?}").contains("larger than"), "{err:?}");
    }

    // --- mirror scheme ---

    #[test]
    fn only_https_and_file_mirrors_are_accepted() {
        assert!(check_base_url("https://mirror.example/releases").is_ok());
        assert!(check_base_url("file:///tmp/release").is_ok());
        for bad in [
            "http://mirror.example",
            "ftp://x",
            "mirror.example",
            "HTTP://x",
        ] {
            let err = check_base_url(bad).unwrap_err();
            assert!(err.contains(BASE_URL_ENV), "{err}");
            assert!(err.contains("https://"), "{err}");
            assert!(!err.contains('\n'), "{err}");
        }
    }

    // --- extraction: links, odd names, corrupt archives ---

    fn tar_gz_link(name: &str, kind: tar::EntryType, link: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_entry_type(kind);
        header.set_link_name(link).unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&builder.into_inner().unwrap()).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn a_tar_symlink_or_hardlink_named_like_the_binary_is_skipped() {
        for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
            let archive = tar_gz_link("d/branch-tally", kind, "/etc/passwd");
            let err = extract_binary("a.tar.gz", &archive, TARGET, CAP).unwrap_err();
            assert!(err.contains("does not contain"), "{kind:?}: {err}");
        }
    }

    #[test]
    fn absolute_backslash_and_drive_names_are_ignored_in_tar_and_zip() {
        let decoy: &[u8] = b"decoy";
        let entries: [(&str, &[u8]); 5] = [
            ("/branch-tally", decoy),
            ("\\branch-tally", decoy),
            ("..\\branch-tally", decoy),
            ("C:\\branch-tally", decoy),
            ("C:branch-tally", decoy),
        ];
        for result in [
            extract_binary("a.tar.gz", &tar_gz(&entries), TARGET, CAP),
            extract_binary("a.zip", &zip_of(&entries), TARGET, CAP),
        ] {
            assert!(result.unwrap_err().contains("does not contain"));
        }
    }

    /// Bytes that do not compress, so truncating the archive cuts through the entry.
    fn noise(len: usize) -> Vec<u8> {
        let mut x = 0x2545_f491_u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.to_le_bytes()[0]
            })
            .collect()
    }

    #[test]
    fn corrupt_archives_fail_and_install_nothing() {
        let big = noise(8000);
        let tar = tar_gz(&[("d/branch-tally", &big)]);
        let zip = zip_of(&[("d/branch-tally", &big)]);
        let corrupt = [
            ("a.tar.gz", tar[..tar.len() / 2].to_vec()),
            ("a.zip", zip[..zip.len() / 2].to_vec()),
        ];
        for (asset, bytes) in corrupt {
            let dir = tempfile::tempdir().unwrap();
            let exe = dir.path().join(BIN_NAME);
            std::fs::write(&exe, b"old").unwrap();
            let installer = FsInstaller {
                exe: exe.clone(),
                target: TARGET,
            };
            assert!(installer.install(asset, &bytes).is_err(), "{asset}");
            assert_eq!(std::fs::read(&exe).unwrap(), b"old");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    // --- replacement hardening ---

    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_at_the_staging_name_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let planted = dir
            .path()
            .join(format!(".branch-tally-new-{}", std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).unwrap();
        let _ = replace_binary(&exe, GOOD, false);
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert_eq!(std::fs::read(&exe).unwrap(), GOOD);
    }

    #[cfg(unix)]
    #[test]
    fn the_existing_permission_bits_are_kept_and_0755_is_the_fallback() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        replace_binary(&exe, GOOD, false).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&exe), 0o700);
        let fresh = dir.path().join("fresh");
        replace_binary(&fresh, GOOD, false).unwrap();
        assert_eq!(mode(&fresh), 0o755);
    }

    #[test]
    fn a_failed_replace_leaves_no_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let err = replace_binary_with(&exe, GOOD, false, &|_, _| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert!(err.contains("could not replace"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_windows_swap_moves_the_old_binary_aside_and_clears_a_stale_one() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        let old = dir.path().join(format!("{BIN_NAME}.old"));
        std::fs::write(&exe, b"v1").unwrap();
        std::fs::write(&old, b"v0").unwrap();
        replace_binary(&exe, GOOD, true).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), GOOD);
        assert_eq!(std::fs::read(&old).unwrap(), b"v1");
    }

    #[test]
    fn a_stale_old_file_that_cannot_be_removed_is_named_as_the_problem() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"v1").unwrap();
        // A non-empty directory cannot be removed as a file, standing in for a running `.old`.
        let old = dir.path().join(format!("{BIN_NAME}.old"));
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("x"), b"x").unwrap();
        let err = replace_binary(&exe, GOOD, true).unwrap_err();
        assert!(err.contains("stale"), "{err}");
        assert!(err.contains(".old"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"v1");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_failed_swap_restores_the_old_binary() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"v1").unwrap();
        let calls = std::cell::Cell::new(0);
        let err = replace_binary_with(&exe, GOOD, true, &|from, to| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert!(err.contains("could not move the new binary"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"v1");
    }

    #[test]
    fn when_the_restore_also_fails_the_error_says_where_the_binary_is() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"v1").unwrap();
        let calls = std::cell::Cell::new(0);
        let err = replace_binary_with(&exe, GOOD, true, &|from, to| {
            calls.set(calls.get() + 1);
            if calls.get() >= 2 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        let old = format!("{BIN_NAME}.old");
        assert!(err.contains("moved to"), "{err}");
        assert!(err.contains(&old), "{err}");
        assert!(err.contains("rename"), "{err}");
        assert_eq!(std::fs::read(dir.path().join(&old)).unwrap(), b"v1");
    }
}
