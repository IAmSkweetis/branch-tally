//! The `update` subcommand: the real release client, installer and confirmer behind core's traits.
//!
//! Core decides what to do ([`branch_tally_core::update::run`]); this file only talks to the
//! network, the filesystem and the terminal.

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use branch_tally_core::update::{
    Confirmer, FetchError, Installer, ReleaseClient, SUMS_NAME, UpdateOptions, run as run_update,
};

use crate::cli::{UpdateArgs, VERSION};

/// Release mirror, same variable and default as `install.sh` and `install.ps1`. Also lets the
/// tests point the client at a `file://` fake release.
const BASE_URL_ENV: &str = "BRANCH_TALLY_INSTALL_BASE_URL";
const DEFAULT_BASE_URL: &str = "https://github.com/IAmSkweetis/branch-tally/releases";

/// The largest download accepted. Release archives are a few MB; this only stops a runaway body.
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// The binary's file name inside the archive and on disk.
const BIN_NAME: &str = if cfg!(windows) {
    "branch-tally.exe"
} else {
    "branch-tally"
};

/// The release target this binary was built for, or a value core reports as unsupported.
fn target_triple() -> String {
    let known = if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "musl"
    )) {
        Some("x86_64-unknown-linux-musl")
    } else if cfg!(all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "musl"
    )) {
        Some("aarch64-unknown-linux-musl")
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(
        target_arch = "aarch64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        Some("aarch64-pc-windows-msvc")
    } else {
        None
    };
    known.map_or_else(
        || {
            format!(
                "{}-{}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS,
                if cfg!(target_env = "gnu") {
                    "gnu"
                } else {
                    "other"
                }
            )
        },
        str::to_owned,
    )
}

/// Run `update`. `Ok` is the line for stdout; `Err` is the line for stderr (exit 1).
pub(crate) fn run(args: &UpdateArgs) -> Result<String, String> {
    let exe = current_exe();
    if cfg!(windows) {
        remove_leftover(&exe);
    }
    let base = std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let options = UpdateOptions {
        check: args.check,
        version: args.version.clone(),
        yes: args.yes,
        force: args.force,
    };
    let outcome = run_update(
        VERSION,
        &target_triple(),
        &options,
        &HttpClient::new(&base),
        &FsInstaller { exe },
        &TerminalConfirmer,
    )
    .map_err(|e| e.to_string())?;
    if outcome.is_refusal() {
        Err(outcome.message())
    } else {
        Ok(outcome.message())
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
}

impl HttpClient {
    fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    fn fetch(&self, path: &str) -> Result<Vec<u8>, FetchError> {
        match self.base.strip_prefix("file://") {
            Some(dir) => read_local(dir, path),
            None => fetch_http(&format!("{}/{path}", self.base)),
        }
    }
}

impl ReleaseClient for HttpClient {
    fn latest_tag(&self) -> Result<String, FetchError> {
        let sums = self.fetch(&format!("latest/download/{SUMS_NAME}"))?;
        tag_from_sums(&String::from_utf8_lossy(&sums))
            .ok_or_else(|| FetchError::Failed("the latest release lists no archives".to_owned()))
    }

    fn download(&self, tag: &str, asset: &str) -> Result<Vec<u8>, FetchError> {
        self.fetch(&format!("download/{tag}/{asset}"))
    }
}

/// Read `path` (slash-separated, relative) under a `file://` directory.
fn read_local(dir: &str, path: &str) -> Result<Vec<u8>, FetchError> {
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
    std::fs::read(file).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => FetchError::NotFound,
        kind => FetchError::Failed(format!("could not read the release files: {kind}")),
    })
}

fn fetch_http(url: &str) -> Result<Vec<u8>, FetchError> {
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
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| FetchError::Failed(format!("could not read the response: {}", e.kind())))?;
    Ok(bytes)
}

/// The release tag named by the first archive in a `SHA256SUMS` body (`branch-tally-v1.2.3-...`).
fn tag_from_sums(sums: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let name = line.split_whitespace().nth(1)?.trim_start_matches('*');
        let rest = name.strip_prefix("branch-tally-")?;
        let (tag, _) = rest.split_once('-')?;
        tag.starts_with('v').then(|| tag.to_owned())
    })
}

// ---------------------------------------------------------------------------------------------
// Installer

/// Replaces the executable at `exe`.
struct FsInstaller {
    exe: PathBuf,
}

impl Installer for FsInstaller {
    fn check_writable(&self) -> Result<(), String> {
        check_dir_writable(&self.exe)
    }

    fn install(&self, asset: &str, archive: &[u8]) -> Result<(), String> {
        let bytes = extract_binary(asset, archive)?;
        replace_binary(&self.exe, &bytes)
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
fn extract_binary(asset: &str, archive: &[u8]) -> Result<Vec<u8>, String> {
    let found = if asset.ends_with(".tar.gz") {
        extract_from_tar_gz(archive)?
    } else if Path::new(asset)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        extract_from_zip(archive)?
    } else {
        return Err("unknown archive type".to_owned());
    };
    found.ok_or_else(|| format!("the archive does not contain {BIN_NAME}"))
}

/// True when `path` (either separator) is a plain relative path whose last part is the binary.
fn is_the_binary(path: &str) -> bool {
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    parts.last() == Some(&BIN_NAME)
        && !parts.iter().any(|p| *p == ".." || p.contains(':'))
        && !path.starts_with(['/', '\\'])
}

fn read_capped(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not read the archive: {}", e.kind()))?;
    Ok(bytes)
}

fn extract_from_tar_gz(archive: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let unreadable = |e: io::Error| format!("could not read the archive: {}", e.kind());
    for entry in tar.entries().map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if is_the_binary(&path) {
            return read_capped(entry).map(Some);
        }
    }
    Ok(None)
}

fn extract_from_zip(archive: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let unreadable = |e: zip::result::ZipError| format!("could not read the archive: {e}");
    let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(unreadable)?;
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(unreadable)?;
        if file.is_file() && file.name().is_ok_and(|name| is_the_binary(&name)) {
            return read_capped(file).map(Some);
        }
    }
    Ok(None)
}

/// Write `bytes` beside `exe` and move it into place so the old binary is never half-replaced.
fn replace_binary(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |what: &str, e: io::Error| format!("{what}: {}", e.kind());
    let dir = exe.parent().ok_or("the binary has no parent directory")?;
    let staged = dir.join(format!(".branch-tally-new-{}", std::process::id()));
    let result = (|| {
        std::fs::write(&staged, bytes).map_err(|e| fail("could not write the new binary", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| fail("could not mark the new binary executable", e))?;
        }
        if cfg!(windows) {
            // A running exe cannot be overwritten but can be renamed.
            let old = old_path(exe);
            let _ = std::fs::remove_file(&old);
            std::fs::rename(exe, &old)
                .map_err(|e| fail("could not move the old binary aside", e))?;
            if let Err(e) = std::fs::rename(&staged, exe) {
                let _ = std::fs::rename(&old, exe);
                return Err(fail("could not move the new binary into place", e));
            }
            Ok(())
        } else {
            std::fs::rename(&staged, exe).map_err(|e| fail("could not replace the binary", e))
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

/// `branch-tally.exe` becomes `branch-tally.exe.old`.
fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Best-effort removal of the `.old` file a previous Windows update left behind.
fn remove_leftover(exe: &Path) {
    let _ = std::fs::remove_file(old_path(exe));
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
    use std::fmt::Write as _;

    const TARGET: &str = "x86_64-unknown-linux-musl";
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

    /// SHA-256, written out here because the binary crate has no digest dependency.
    #[expect(
        clippy::many_single_char_names,
        clippy::too_many_lines,
        reason = "the SHA-256 spec names its working variables a to h"
    )]
    fn sha256_hex(data: &[u8]) -> String {
        const K: [u32; 64] = [
            0x428a_2f98,
            0x7137_4491,
            0xb5c0_fbcf,
            0xe9b5_dba5,
            0x3956_c25b,
            0x59f1_11f1,
            0x923f_82a4,
            0xab1c_5ed5,
            0xd807_aa98,
            0x1283_5b01,
            0x2431_85be,
            0x550c_7dc3,
            0x72be_5d74,
            0x80de_b1fe,
            0x9bdc_06a7,
            0xc19b_f174,
            0xe49b_69c1,
            0xefbe_4786,
            0x0fc1_9dc6,
            0x240c_a1cc,
            0x2de9_2c6f,
            0x4a74_84aa,
            0x5cb0_a9dc,
            0x76f9_88da,
            0x983e_5152,
            0xa831_c66d,
            0xb003_27c8,
            0xbf59_7fc7,
            0xc6e0_0bf3,
            0xd5a7_9147,
            0x06ca_6351,
            0x1429_2967,
            0x27b7_0a85,
            0x2e1b_2138,
            0x4d2c_6dfc,
            0x5338_0d13,
            0x650a_7354,
            0x766a_0abb,
            0x81c2_c92e,
            0x9272_2c85,
            0xa2bf_e8a1,
            0xa81a_664b,
            0xc24b_8b70,
            0xc76c_51a3,
            0xd192_e819,
            0xd699_0624,
            0xf40e_3585,
            0x106a_a070,
            0x19a4_c116,
            0x1e37_6c08,
            0x2748_774c,
            0x34b0_bcb5,
            0x391c_0cb3,
            0x4ed8_aa4a,
            0x5b9c_ca4f,
            0x682e_6ff3,
            0x748f_82ee,
            0x78a5_636f,
            0x84c8_7814,
            0x8cc7_0208,
            0x90be_fffa,
            0xa450_6ceb,
            0xbef9_a3f7,
            0xc671_78f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
        for block in msg.chunks(64) {
            let mut w = [0u32; 64];
            for (i, word) in block.chunks(4).enumerate() {
                w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *slot = slot.wrapping_add(v);
            }
        }
        h.iter().fold(String::new(), |mut hex, v| {
            let _ = write!(hex, "{v:08x}");
            hex
        })
    }

    #[test]
    fn the_test_sha256_matches_a_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A fake release directory holding `v1.1.0` for [`TARGET`], plus a `latest` alias of it.
    fn fake_release(dir: &Path, archive: &[u8]) -> String {
        let asset = format!("branch-tally-v1.1.0-{TARGET}.tar.gz");
        let sums = format!(
            "{}  {asset}\n{}  other-asset.zip\n",
            sha256_hex(archive),
            sha256_hex(b"x")
        );
        for sub in ["download/v1.1.0", "latest/download"] {
            let d = dir.join(sub);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(SUMS_NAME), &sums).unwrap();
            std::fs::write(d.join(&asset), archive).unwrap();
        }
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
            extract_binary("branch-tally-v1.1.0-x.tar.gz", &archive).unwrap(),
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
            extract_binary("branch-tally-v1.1.0-x.zip", &archive).unwrap(),
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
        assert_eq!(extract_binary("a.tar.gz", &tar_gz(&entries)).unwrap(), GOOD);
        assert_eq!(extract_binary("a.zip", &zip_of(&entries)).unwrap(), GOOD);
    }

    #[test]
    fn an_archive_without_the_binary_is_an_error() {
        let archive = tar_gz(&[("dir/LICENSE", b"license")]);
        assert!(extract_binary("a.tar.gz", &archive).is_err());
        assert!(extract_binary("a.tar.gz", b"not an archive").is_err());
        assert!(extract_binary("a.rar", b"").is_err());
    }

    // --- replacement ---

    #[test]
    fn replacing_leaves_a_working_file_at_the_target_and_no_stragglers() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        replace_binary(&exe, GOOD).unwrap();
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
        assert!(replace_binary(&exe, GOOD).is_err());
        assert!(exe.join("keep").exists());
    }

    #[test]
    fn remove_leftover_deletes_the_old_file_and_tolerates_its_absence() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join(BIN_NAME);
        let old = dir.path().join(format!("{BIN_NAME}.old"));
        std::fs::write(&old, b"old").unwrap();
        remove_leftover(&exe);
        assert!(!old.exists());
        remove_leftover(&exe);
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

    // --- client, through core's run against a file:// release ---

    fn update(base: &str, exe: &Path, options: &UpdateOptions) -> Result<Outcome, UpdateError> {
        run_update(
            "1.0.0",
            TARGET,
            options,
            &HttpClient::new(base),
            &FsInstaller {
                exe: exe.to_path_buf(),
            },
            &Yes,
        )
    }

    #[test]
    fn updates_from_a_fake_release_to_the_latest_tag() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), &release_archive());
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let outcome = update(
            &base,
            &exe,
            &UpdateOptions {
                yes: true,
                ..UpdateOptions::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.message(), "updated branch-tally v1.0.0 -> v1.1.0");
        assert_eq!(std::fs::read(&exe).unwrap(), GOOD);
    }

    #[test]
    fn check_reports_the_latest_without_touching_the_binary() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), &release_archive());
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let outcome = update(
            &base,
            &exe,
            &UpdateOptions {
                check: true,
                ..UpdateOptions::default()
            },
        )
        .unwrap();
        assert!(outcome.message().contains("v1.1.0"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn an_unknown_tag_is_a_not_found_message() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), &release_archive());
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let options = UpdateOptions {
            yes: true,
            version: Some("v9.9.9".to_owned()),
            ..UpdateOptions::default()
        };
        let err = update(&base, &exe, &options).unwrap_err();
        assert!(err.to_string().contains("release not found"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn a_tampered_archive_installs_nothing() {
        let release = tempfile::tempdir().unwrap();
        let base = fake_release(release.path(), &release_archive());
        let asset = format!("branch-tally-v1.1.0-{TARGET}.tar.gz");
        std::fs::write(
            release.path().join("download/v1.1.0").join(asset),
            b"tampered",
        )
        .unwrap();
        let bin = tempfile::tempdir().unwrap();
        let exe = bin.path().join(BIN_NAME);
        std::fs::write(&exe, b"old").unwrap();
        let err = update(
            &base,
            &exe,
            &UpdateOptions {
                yes: true,
                ..UpdateOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn the_tag_comes_from_the_first_archive_in_the_sums() {
        let sums = "abc  install.sh\nabc  branch-tally-v2.10.3-x86_64-apple-darwin.tar.gz\n";
        assert_eq!(tag_from_sums(sums).as_deref(), Some("v2.10.3"));
        assert_eq!(tag_from_sums("abc  install.sh\n"), None);
    }

    #[test]
    fn a_missing_file_release_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let client = HttpClient::new(&format!("file://{}", dir.path().to_string_lossy()));
        assert_eq!(client.download("v1.0.0", "x"), Err(FetchError::NotFound));
    }

    #[test]
    fn the_target_is_never_empty() {
        assert_ne!(target_triple(), "");
    }
}
