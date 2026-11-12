//! The guarded `.tally/` accessor (security assessment F-01, #94).
//!
//! `.tally/` lives in the repository, and git tracks symbolic links, so a checkout can ship
//! `.tally/captures.jsonl` as a link to a file elsewhere, or `.tally` itself as a link. Everything
//! core reads or writes under the default `.tally/` goes through [`TallyDir`], which refuses links
//! rather than following them, and does nothing at all when the repository has not opted in by
//! creating `.tally/`.
//!
//! The rules:
//!
//! - `.tally` is checked once, in [`TallyDir::open`]: absent is `Ok(None)` and nothing is created;
//!   a symlink or a non-directory is refused. The canonical parent of `.tally` must be the
//!   canonical main checkout.
//! - A file name must be a single plain component (no separators, `.`, `..`, or empty).
//! - A target file is checked without following links and must be a regular file. A missing
//!   target may be created for append. After opening, the opened handle is re-checked to be a
//!   regular file.
//!
//! # Guarantees
//!
//! - **Unix**: `TallyDir` holds an owned directory fd for `.tally`, opened with `O_NOFOLLOW |
//!   O_DIRECTORY` and verified with fstat. Every file is opened with `openat` relative to that fd
//!   and `O_NOFOLLOW`. Swapping `.tally` for a symlink after `open` therefore changes nothing: the
//!   fd still names the original directory, so reads and appends keep going to the original
//!   (possibly renamed) directory and never to the link target.
//! - **Windows**: `TallyDir` holds a handle on `.tally` opened with backup semantics and
//!   open-reparse-point, shared for read and write but not delete, and verified to be a directory
//!   and not a reparse point. While it is held `.tally` cannot be renamed or deleted, so it cannot
//!   be replaced by a link. Files are still opened by path (with open-reparse-point and a
//!   handle re-check), which is safe only because of that held handle.
//!
//! # Limits
//!
//! The path above `.tally` (the checkout root) is the trust anchor on both platforms: if it is
//! itself attacker-controlled or swapped while `open` runs, that is out of scope. Within `open`,
//! the directory handle is opened after the first check and re-verified, but the containment
//! check uses the path, so it is not atomic with the handle.
//!
//! An explicit user-supplied `--ledger <path>` does not come through here: it keeps the plain
//! path-taking functions. Errors name a path, never content.

use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::LedgerError;

/// The directory name, relative to the main checkout.
pub const TALLY_DIR: &str = ".tally";

/// A verified `.tally/` directory of a main checkout.
#[derive(Debug, Clone)]
pub struct TallyDir {
    dir: PathBuf,
    /// Unix: the directory fd files are opened relative to. Windows: a no-delete-share handle
    /// that pins `.tally` in place.
    handle: Arc<File>,
}

fn refused(path: &Path, reason: &'static str) -> LedgerError {
    LedgerError::Refused {
        path: path.to_owned(),
        reason,
    }
}

/// Whether metadata is not a Windows reparse point (always true elsewhere).
fn not_reparse(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x0400 == 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        true
    }
}

/// Whether metadata describes a regular file and not a reparse point.
fn is_plain_file(meta: &Metadata) -> bool {
    not_reparse(meta) && meta.file_type().is_file()
}

/// Whether a name is a single plain component.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0', ':'])
}

/// Opens the `.tally` directory itself, never following a final link.
#[cfg(unix)]
fn open_dir_handle(dir: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok(File::from(fd))
}

#[cfg(windows)]
fn open_dir_handle(dir: &Path) -> std::io::Result<File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_BACKUP_SEMANTICS (open a directory) | FILE_FLAG_OPEN_REPARSE_POINT.
    // Share read and write, deliberately not delete: `.tally` cannot be renamed or removed
    // while this handle lives.
    OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(dir)
}

/// Maps a failed open of the `.tally` handle: a link or non-directory raced in is a refusal.
fn handle_open_error(dir: &Path, source: std::io::Error) -> LedgerError {
    #[cfg(unix)]
    if matches!(
        source.raw_os_error(),
        Some(code) if code == rustix::io::Errno::LOOP.raw_os_error()
            || code == rustix::io::Errno::NOTDIR.raw_os_error()
    ) {
        return refused(dir, "it is a symbolic link or not a directory");
    }
    LedgerError::Read {
        path: dir.to_owned(),
        source,
    }
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
        if meta.file_type().is_symlink() || !not_reparse(&meta) {
            return Err(refused(&dir, "it is a symbolic link"));
        }
        if !meta.file_type().is_dir() {
            return Err(refused(&dir, "it is not a directory"));
        }
        let handle = match open_dir_handle(&dir) {
            Ok(h) => h,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(handle_open_error(&dir, source)),
        };
        let handle_meta = handle.metadata().map_err(|source| LedgerError::Read {
            path: dir.clone(),
            source,
        })?;
        if !not_reparse(&handle_meta) {
            return Err(refused(&dir, "it is a symbolic link"));
        }
        if !handle_meta.file_type().is_dir() {
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
        Ok(Some(Self {
            dir,
            handle: Arc::new(handle),
        }))
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

    /// Checks `name` without following a link: `Ok(None)` when missing, `Ok(Some(is_plain))`
    /// otherwise.
    fn probe(&self, name: &str) -> std::io::Result<Option<bool>> {
        #[cfg(unix)]
        {
            use rustix::fs::{AtFlags, FileType};
            match rustix::fs::statat(&*self.handle, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => Ok(Some(
                    FileType::from_raw_mode(st.st_mode) == FileType::RegularFile,
                )),
                Err(e) if e == rustix::io::Errno::NOENT => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
        #[cfg(windows)]
        {
            match std::fs::symlink_metadata(self.file_path(name)) {
                Ok(m) => Ok(Some(is_plain_file(&m))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }
    }

    /// Opens `name` without following a final link, for reading or appending.
    fn open_file(&self, name: &str, append: bool) -> std::io::Result<File> {
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags};
            // O_NONBLOCK so a FIFO raced in after the check cannot hang the open.
            let mut flags = OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
            flags |= if append {
                OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE
            } else {
                OFlags::RDONLY
            };
            let fd = rustix::fs::openat(&*self.handle, name, flags, Mode::from_raw_mode(0o666))?;
            Ok(File::from(fd))
        }
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            use std::os::windows::fs::OpenOptionsExt;
            let mut options = OpenOptions::new();
            if append {
                options.create(true).append(true);
            } else {
                options.read(true);
            }
            // FILE_FLAG_OPEN_REPARSE_POINT: open a link itself instead of its target.
            options.custom_flags(0x0020_0000);
            options.open(self.file_path(name))
        }
    }

    /// Reads `name`. `Ok(None)` when the file does not exist.
    ///
    /// # Errors
    /// [`LedgerError::Refused`] when it is a link or not a regular file, or `name` is not a
    /// single plain component; [`LedgerError::Read`] when it cannot be read.
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, LedgerError> {
        self.read_with(name, &|| {})
    }

    /// `read` with a seam that runs after `.tally` is verified and before the file is opened.
    fn read_with(&self, name: &str, between: &dyn Fn()) -> Result<Option<Vec<u8>>, LedgerError> {
        let path = self.file_path(name);
        if !is_plain_name(name) {
            return Err(refused(&path, "the name is not a single plain component"));
        }
        match self.probe(name) {
            Ok(Some(true)) => {}
            Ok(Some(false)) => return Err(refused(&path, "it is not a regular file")),
            Ok(None) => return Ok(None),
            Err(source) => return Err(LedgerError::Read { path, source }),
        }
        let read_err = |source| LedgerError::Read {
            path: path.clone(),
            source,
        };
        between();
        let mut file = self.open_file(name, false).map_err(read_err)?;
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
    /// [`LedgerError::Refused`] when it is a link or not a regular file, or `name` is not a
    /// single plain component; [`LedgerError::Write`] when it cannot be written.
    pub fn append(&self, name: &str, bytes: &[u8]) -> Result<(), LedgerError> {
        self.append_with(name, bytes, &|| {})
    }

    /// `append` with the same seam as `read_with`.
    fn append_with(&self, name: &str, bytes: &[u8], between: &dyn Fn()) -> Result<(), LedgerError> {
        let path = self.file_path(name);
        if !is_plain_name(name) {
            return Err(refused(&path, "the name is not a single plain component"));
        }
        let write_err = |source| LedgerError::Write {
            path: path.clone(),
            source,
        };
        match self.probe(name) {
            Ok(Some(true) | None) => {}
            Ok(Some(false)) => return Err(refused(&path, "it is not a regular file")),
            Err(source) => return Err(write_err(source)),
        }
        between();
        let mut file = self.open_file(name, true).map_err(write_err)?;
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
            skip_or_panic("creating symlinks needs a privilege this machine lacks");
            return;
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
            skip_or_panic("creating symlinks needs a privilege this machine lacks");
            return;
        }
        assert!(TallyDir::open(&repo).is_err());
    }

    /// Swaps the real `.tally` for a symlink to `victim`, returning where the real one went.
    #[cfg(unix)]
    fn swap_tally_for_link(root: &Path, victim: &Path) -> PathBuf {
        let moved = root.join("moved-tally");
        std::fs::rename(root.join(TALLY_DIR), &moved).unwrap();
        std::os::unix::fs::symlink(victim, root.join(TALLY_DIR)).unwrap();
        moved
    }

    #[cfg(unix)]
    #[test]
    fn append_after_tally_is_swapped_for_a_link_does_not_reach_the_victim() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let victim = dir.path().join("victim-dir");
        std::fs::create_dir(&victim).unwrap();
        let moved = std::cell::RefCell::new(None);
        // Pinned behaviour: the append goes through the held fd into the moved original.
        let result = tally.append_with("f.jsonl", b"evil\n", &|| {
            *moved.borrow_mut() = Some(swap_tally_for_link(dir.path(), &victim));
        });
        assert_eq!(
            std::fs::read_dir(&victim).unwrap().count(),
            0,
            "victim written"
        );
        assert!(result.is_ok(), "{result:?}");
        let moved = moved.borrow().clone().unwrap();
        assert_eq!(std::fs::read(moved.join("f.jsonl")).unwrap(), b"evil\n");
    }

    #[cfg(unix)]
    #[test]
    fn read_after_tally_is_swapped_for_a_link_does_not_return_the_victims_file() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        tally.append("f.jsonl", b"mine\n").unwrap();
        let victim = dir.path().join("victim-dir");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("f.jsonl"), b"theirs\n").unwrap();
        // Pinned behaviour: the read goes through the held fd and returns the original's bytes.
        let got = tally
            .read_with("f.jsonl", &|| {
                swap_tally_for_link(dir.path(), &victim);
            })
            .unwrap();
        assert_eq!(got.unwrap(), b"mine\n");
    }

    #[test]
    fn names_that_are_not_a_single_plain_component_are_refused() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        for name in ["", ".", "..", "a/b", "../x", "a\\b"] {
            assert!(
                matches!(tally.read(name), Err(LedgerError::Refused { .. })),
                "read {name:?}"
            );
            assert!(
                matches!(tally.append(name, b"x"), Err(LedgerError::Refused { .. })),
                "append {name:?}"
            );
        }
    }

    /// On Windows CI a skipped test would hide a missing guarantee, so it fails there; locally
    /// (no `CI` variable) it skips.
    #[cfg(windows)]
    fn skip_or_panic(reason: &str) {
        assert!(
            std::env::var_os("CI").is_none(),
            "test cannot run on CI: {reason}"
        );
    }

    /// Creates a junction `link` -> `target`; junctions need no privilege.
    #[cfg(windows)]
    fn make_junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_at_tally_is_refused_and_the_victim_untouched() {
        let dir = checkout();
        let victim = dir.path().join("victim-dir");
        std::fs::create_dir(&victim).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        if !make_junction(&repo.join(TALLY_DIR), &victim) {
            skip_or_panic("mklink /J failed");
            return;
        }
        let err = TallyDir::open(&repo).unwrap_err();
        assert!(matches!(err, LedgerError::Refused { .. }), "{err}");
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_as_the_target_name_is_refused_and_the_victim_untouched() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let victim = dir.path().join("victim-dir");
        std::fs::create_dir(&victim).unwrap();
        if !make_junction(&tally.file_path("f.jsonl"), &victim) {
            skip_or_panic("mklink /J failed");
            return;
        }
        assert!(tally.append("f.jsonl", b"evil\n").is_err());
        assert!(tally.read("f.jsonl").is_err());
        assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
    }

    /// The held handle has no `FILE_SHARE_DELETE`, so `.tally` cannot be renamed away between the
    /// check and the open. Cannot run on Linux; it compiles only on Windows.
    #[cfg(windows)]
    #[test]
    fn tally_cannot_be_renamed_while_a_tally_dir_is_held() {
        let dir = checkout();
        let tally = with_tally(dir.path());
        let victim = dir.path().join("victim-dir");
        std::fs::create_dir(&victim).unwrap();
        let renamed = std::cell::Cell::new(false);
        let result = tally.append_with("f.jsonl", b"evil\n", &|| {
            let moved = dir.path().join("moved-tally");
            if std::fs::rename(dir.path().join(TALLY_DIR), &moved).is_ok() {
                renamed.set(true);
                let _ = make_junction(&dir.path().join(TALLY_DIR), &victim);
            }
        });
        if renamed.get() {
            // If the rename was somehow allowed, the victim must still be untouched.
            assert_eq!(std::fs::read_dir(&victim).unwrap().count(), 0);
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
    }
}
