//! The guarded `.tally/` accessor (security assessment F-01, #94).
//!
//! `.tally/` lives in the repository, and git tracks symbolic links, so a checkout can ship
//! `.tally/captures.jsonl` as a link to a file elsewhere. Everything core reads or writes under
//! the default `.tally/` goes through [`TallyDir`], which refuses links rather than following
//! them, and does nothing at all when the repository has not opted in by creating `.tally/`.
//!
//! The rules:
//!
//! - `.tally` is checked with `symlink_metadata`: absent is `Ok(None)` and nothing is created;
//!   a symlink or a non-directory is refused.
//! - The canonical parent of `.tally` must be the canonical main checkout.
//! - A target file is checked with `symlink_metadata` and must be a regular file. A missing
//!   target may be created for append, with create plus no-follow, so a link raced in between the
//!   check and the open is not followed.
//! - Files are opened no-follow (`O_NOFOLLOW` on Unix, `FILE_FLAG_OPEN_REPARSE_POINT` on
//!   Windows), and the opened handle's metadata is re-checked to be a regular file.
//!
//! An explicit user-supplied `--ledger <path>` does not come through here: it keeps the plain
//! path-taking functions. Errors name a path, never content.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::LedgerError;

/// The directory name, relative to the main checkout.
pub const TALLY_DIR: &str = ".tally";

/// A verified `.tally/` directory of a main checkout.
#[derive(Debug, Clone)]
pub struct TallyDir {
    dir: PathBuf,
}

fn refused(path: &Path, reason: &'static str) -> LedgerError {
    LedgerError::Refused {
        path: path.to_owned(),
        reason,
    }
}

/// Opens `path` without following a final symlink, for reading or appending.
fn open_no_follow(path: &Path, append: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    if append {
        options.create(true).append(true);
    } else {
        options.read(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK so a FIFO raced in after the check cannot hang the open.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself instead of its target.
        options.custom_flags(0x0020_0000);
    }
    options.open(path)
}

/// Whether metadata describes a regular file and, on Windows, not a reparse point.
fn is_plain_file(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x0400 != 0 {
            return false;
        }
    }
    meta.file_type().is_file()
}

impl TallyDir {
    /// Verifies `<main_checkout>/.tally`. `Ok(None)` when it does not exist: the repository has
    /// not opted in, and nothing is created.
    ///
    /// # Errors
    /// [`LedgerError::Refused`] when `.tally` is a symlink, is not a directory, or does not
    /// resolve to a direct child of the main checkout; [`LedgerError::Read`] when it cannot be
    /// inspected.
    pub fn open(main_checkout: &Path) -> Result<Option<Self>, LedgerError> {
        let dir = main_checkout.join(TALLY_DIR);
        let meta = match std::fs::symlink_metadata(&dir) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(LedgerError::Read { path: dir, source }),
        };
        if meta.file_type().is_symlink() {
            return Err(refused(&dir, "it is a symbolic link"));
        }
        if !meta.file_type().is_dir() {
            return Err(refused(&dir, "it is not a directory"));
        }
        let canonical = |path: &Path| {
            std::fs::canonicalize(path).map_err(|source| LedgerError::Read {
                path: path.to_owned(),
                source,
            })
        };
        let real = canonical(&dir)?;
        let root = canonical(main_checkout)?;
        if real.parent() != Some(root.as_path()) {
            return Err(refused(
                &dir,
                "it does not resolve inside the main checkout",
            ));
        }
        Ok(Some(Self { dir }))
    }

    /// The `.tally` directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// The path of a file in the directory.
    #[must_use]
    pub fn file_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Reads `name`. `Ok(None)` when the file does not exist.
    ///
    /// # Errors
    /// [`LedgerError::Refused`] when it is a link or not a regular file;
    /// [`LedgerError::Read`] when it cannot be read.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, LedgerError> {
        let path = self.file_path(name);
        match std::fs::symlink_metadata(&path) {
            Ok(m) if is_plain_file(&m) => {}
            Ok(_) => return Err(refused(&path, "it is not a regular file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(LedgerError::Read { path, source }),
        }
        let read_err = |source| LedgerError::Read {
            path: path.clone(),
            source,
        };
        let mut file = open_no_follow(&path, false).map_err(read_err)?;
        let meta = file.metadata().map_err(read_err)?;
        if !is_plain_file(&meta) {
            return Err(refused(&path, "it is not a regular file"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(read_err)?;
        Ok(Some(bytes))
    }

    /// Appends `bytes` to `name`, creating it when missing.
    ///
    /// # Errors
    /// [`LedgerError::Refused`] when it is a link or not a regular file;
    /// [`LedgerError::Write`] when it cannot be written.
    pub fn append(&self, name: &str, bytes: &[u8]) -> Result<(), LedgerError> {
        let path = self.file_path(name);
        let write_err = |source| LedgerError::Write {
            path: path.clone(),
            source,
        };
        match std::fs::symlink_metadata(&path) {
            Ok(m) if is_plain_file(&m) => {}
            Ok(_) => return Err(refused(&path, "it is not a regular file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(write_err(source)),
        }
        let mut file = open_no_follow(&path, true).map_err(write_err)?;
        let meta = file.metadata().map_err(write_err)?;
        if !is_plain_file(&meta) {
            return Err(refused(&path, "it is not a regular file"));
        }
        file.write_all(bytes).map_err(write_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn with_tally(root: &Path) -> TallyDir {
        std::fs::create_dir(root.join(TALLY_DIR)).expect("mkdir");
        TallyDir::open(root).expect("open").expect("present")
    }

    #[test]
    fn absent_tally_is_none_and_creates_nothing() {
        let dir = checkout();
        assert!(TallyDir::open(dir.path()).unwrap().is_none());
        assert!(!dir.path().join(TALLY_DIR).exists());
    }

    #[test]
    fn a_file_named_tally_is_refused() {
        let dir = checkout();
        std::fs::write(dir.path().join(TALLY_DIR), b"x").unwrap();
        let err = TallyDir::open(dir.path()).unwrap_err();
        assert!(matches!(err, LedgerError::Refused { .. }), "{err}");
    }

    #[test]
    fn missing_file_reads_as_none_and_append_creates_it() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        assert_eq!(tally.read("f.jsonl").unwrap(), None);
        tally.append("f.jsonl", b"a\n").unwrap();
        tally.append("f.jsonl", b"b\n").unwrap();
        assert_eq!(tally.read("f.jsonl").unwrap().unwrap(), b"a\nb\n");
    }

    #[test]
    fn a_directory_at_the_target_name_is_refused() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        std::fs::create_dir(tally.file_path("f.jsonl")).unwrap();
        assert!(matches!(
            tally.read("f.jsonl").unwrap_err(),
            LedgerError::Refused { .. }
        ));
        assert!(matches!(
            tally.append("f.jsonl", b"x").unwrap_err(),
            LedgerError::Refused { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_at_the_target_name_is_refused_without_blocking() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let fifo = tally.file_path("f.jsonl");
        let status = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !status.is_ok_and(|s| s.success()) {
            return; // no mkfifo on this machine: nothing to test
        }
        assert!(matches!(
            tally.read("f.jsonl").unwrap_err(),
            LedgerError::Refused { .. }
        ));
        assert!(matches!(
            tally.append("f.jsonl", b"x").unwrap_err(),
            LedgerError::Refused { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_target_is_refused_and_the_victim_untouched() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        std::os::unix::fs::symlink(&victim, tally.file_path("f.jsonl")).unwrap();
        assert!(matches!(
            tally.append("f.jsonl", b"evil\n").unwrap_err(),
            LedgerError::Refused { .. }
        ));
        assert!(matches!(
            tally.read("f.jsonl").unwrap_err(),
            LedgerError::Refused { .. }
        ));
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_target_is_not_created() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let missing = dir.path().join("not-yet");
        std::os::unix::fs::symlink(&missing, tally.file_path("f.jsonl")).unwrap();
        assert!(tally.append("f.jsonl", b"evil\n").is_err());
        assert!(!missing.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_tally_dir_is_refused() {
        let dir = checkout();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::os::unix::fs::symlink(&elsewhere, repo.join(TALLY_DIR)).unwrap();
        let err = TallyDir::open(&repo).unwrap_err();
        assert!(matches!(err, LedgerError::Refused { .. }), "{err}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_symlinked_target_is_refused_on_windows() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        if std::os::windows::fs::symlink_file(&victim, tally.file_path("f.jsonl")).is_err() {
            return; // creating symlinks needs a privilege this machine lacks
        }
        assert!(tally.append("f.jsonl", b"evil\n").is_err());
        assert!(tally.read("f.jsonl").is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    }

    #[cfg(windows)]
    #[test]
    fn a_symlinked_tally_dir_is_refused_on_windows() {
        let dir = checkout();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        if std::os::windows::fs::symlink_dir(&elsewhere, repo.join(TALLY_DIR)).is_err() {
            return;
        }
        assert!(TallyDir::open(&repo).is_err());
    }
}
