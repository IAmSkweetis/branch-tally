//! Exclusive, atomic file replacement.
//!
//! [`write_atomic`] stages the bytes in a sibling file with an unpredictable name, opened with
//! `create_new` (`O_EXCL` / `CREATE_NEW`), so a file or symlink planted at the temporary name makes
//! the open fail instead of being followed. It then fsyncs and renames over the target. The
//! temporary file is removed on any failure.
//!
//! Replacing the target does not preserve its permissions: the new file gets the process default.
//! The target itself is replaced, not written through, so a symlink at the target is swapped for a
//! regular file rather than followed.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How many fresh names are tried when the chosen one already exists.
pub const MAX_ATTEMPTS: usize = 8;

/// A source of temporary-name suffixes. The real one is the OS CSPRNG; a test supplies a fixed
/// sequence to force a collision.
pub trait SuffixSource {
    /// The next suffix: ASCII letters and digits only, never a path separator.
    ///
    /// # Errors
    /// When the source cannot produce one.
    fn next_suffix(&mut self) -> io::Result<String>;
}

/// 16 random bytes from the operating system's CSPRNG, as lowercase hex.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsSuffix;

impl SuffixSource for OsSuffix {
    fn next_suffix(&mut self) -> io::Result<String> {
        use ring::rand::SecureRandom;
        use std::fmt::Write as _;
        let mut bytes = [0_u8; 16];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| io::Error::other("the system random number generator failed"))?;
        Ok(bytes.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        }))
    }
}

/// Atomically replaces `target` with `bytes`, staging through a random sibling.
///
/// # Errors
/// An [`io::Error`] when the target has no file name, the staging file cannot be created
/// exclusively within [`MAX_ATTEMPTS`] tries, or writing, syncing or renaming fails. The staging
/// file is removed before returning an error.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(target, bytes, &mut OsSuffix)
}

/// [`write_atomic`] with an injected suffix source, so a test can force a name collision.
///
/// # Errors
/// As [`write_atomic`].
pub fn write_atomic_with(
    target: &Path,
    bytes: &[u8],
    suffixes: &mut dyn SuffixSource,
) -> io::Result<()> {
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "the target has no file name")
    })?;
    let dir = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    for _ in 0..MAX_ATTEMPTS {
        let temp = temp_path(dir, name, &suffixes.next_suffix()?);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let staged = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| {
                drop(file);
                fs::rename(&temp, target)
            });
        if let Err(e) = staged {
            // Best effort: the original error is the one worth reporting.
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create an unused temporary file name",
    ))
}

fn temp_path(dir: &Path, name: &OsStr, suffix: &str) -> PathBuf {
    let mut file = OsString::from(".");
    file.push(name);
    file.push(".");
    file.push(suffix);
    file.push(".tmp");
    dir.join(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<&'static str>);
    impl SuffixSource for Fixed {
        fn next_suffix(&mut self) -> io::Result<String> {
            if self.0.is_empty() {
                return Err(io::Error::other("exhausted"));
            }
            Ok(self.0.remove(0).to_owned())
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn writes_new_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        write_atomic(&target, b"one").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"one");
        assert_eq!(names(dir.path()), ["out.json"]);
    }

    #[test]
    fn replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        fs::write(&target, b"old").unwrap();
        write_atomic(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(names(dir.path()), ["out.json"]);
    }

    #[test]
    fn planted_file_at_temp_name_is_not_overwritten_and_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        let planted = temp_path(dir.path(), OsStr::new("out.json"), "aaaa");
        fs::write(&planted, b"planted").unwrap();
        write_atomic_with(&target, b"data", &mut Fixed(vec!["aaaa", "bbbb"])).unwrap();
        assert_eq!(fs::read(&planted).unwrap(), b"planted");
        assert_eq!(fs::read(&target).unwrap(), b"data");
        assert_eq!(names(dir.path()).len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn planted_symlink_at_temp_name_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"precious").unwrap();
        let target = dir.path().join("out.json");
        let planted = temp_path(dir.path(), OsStr::new("out.json"), "aaaa");
        std::os::unix::fs::symlink(&victim, &planted).unwrap();
        write_atomic_with(&target, b"data", &mut Fixed(vec!["aaaa", "bbbb"])).unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"precious");
        assert!(
            fs::symlink_metadata(&planted)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"data");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_at_temp_name_is_not_created_through() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let target = dir.path().join("out.json");
        let planted = temp_path(dir.path(), OsStr::new("out.json"), "aaaa");
        std::os::unix::fs::symlink(&missing, &planted).unwrap();
        write_atomic_with(&target, b"data", &mut Fixed(vec!["aaaa", "bbbb"])).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn gives_up_after_bounded_attempts_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        fs::write(&target, b"old").unwrap();
        let planted = temp_path(dir.path(), OsStr::new("out.json"), "aaaa");
        fs::write(&planted, b"planted").unwrap();
        let mut same = Fixed(vec!["aaaa"; MAX_ATTEMPTS]);
        let err = write_atomic_with(&target, b"new", &mut same).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(same.0.is_empty(), "tried every allowed attempt");
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read(&planted).unwrap(), b"planted");
    }

    #[test]
    fn removes_temp_when_rename_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("inner"), b"x").unwrap();
        assert!(write_atomic(&target, b"data").is_err());
        assert_eq!(names(dir.path()), ["out.json"]);
    }

    #[test]
    fn target_without_file_name_is_an_error() {
        assert!(write_atomic(Path::new("/"), b"x").is_err());
    }

    #[test]
    fn suffix_source_failure_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.json");
        assert!(write_atomic_with(&target, b"x", &mut Fixed(vec![])).is_err());
        assert_eq!(names(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn os_suffixes_are_hex_and_differ() {
        let a = OsSuffix.next_suffix().unwrap();
        let b = OsSuffix.next_suffix().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_target_name_works() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(OsStr::from_bytes(b"out\xff.json"));
        write_atomic(&target, b"data").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"data");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
