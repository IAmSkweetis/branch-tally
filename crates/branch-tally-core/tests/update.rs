//! Orchestration of `update` against fake client, installer and confirmer.

use std::cell::RefCell;

use branch_tally_core::update::{
    Confirmer, FetchError, Installer, Outcome, Refusal, ReleaseClient, UpdateError, UpdateOptions,
    run,
};

const TARGET: &str = "x86_64-unknown-linux-musl";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

const ARCHIVE: &[u8] = b"fake archive bytes";

struct Client {
    latest: Result<String, FetchError>,
    /// Tags that exist.
    tags: Vec<&'static str>,
    sums_override: Option<String>,
    archive: Vec<u8>,
    net_error: Option<FetchError>,
    log: RefCell<Vec<String>>,
}

impl Client {
    fn new(latest: &str) -> Self {
        Self {
            latest: Ok(latest.to_string()),
            tags: vec!["v1.0.0", "v1.2.0", "v1.3.0", "v2.0.0"],
            sums_override: None,
            archive: ARCHIVE.to_vec(),
            net_error: None,
            log: RefCell::default(),
        }
    }
    fn downloads(&self) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|l| l.starts_with("download"))
            .count()
    }
    fn calls(&self) -> usize {
        self.log.borrow().len()
    }
}

impl ReleaseClient for Client {
    fn latest_tag(&self) -> Result<String, FetchError> {
        self.log.borrow_mut().push("latest".into());
        self.latest.clone()
    }
    fn download(&self, tag: &str, asset: &str) -> Result<Vec<u8>, FetchError> {
        self.log
            .borrow_mut()
            .push(format!("download {tag} {asset}"));
        if let Some(e) = &self.net_error {
            return Err(e.clone());
        }
        if !self.tags.contains(&tag) {
            return Err(FetchError::NotFound);
        }
        if asset == "SHA256SUMS" {
            let name = format!("branch-tally-{tag}-{TARGET}.tar.gz");
            return Ok(self
                .sums_override
                .clone()
                .unwrap_or_else(|| format!("{}  {name}\n", hex(ARCHIVE)))
                .into_bytes());
        }
        Ok(self.archive.clone())
    }
}

#[derive(Default)]
struct Inst {
    unwritable: Option<String>,
    install_fails: bool,
    installed: RefCell<Vec<String>>,
}

impl Installer for Inst {
    fn check_writable(&self) -> Result<(), String> {
        self.unwritable.clone().map_or(Ok(()), Err)
    }
    fn install(&self, asset: &str, _archive: &[u8]) -> Result<(), String> {
        if self.install_fails {
            return Err("disk full".into());
        }
        self.installed.borrow_mut().push(asset.to_string());
        Ok(())
    }
}

struct Confirm {
    answer: bool,
    asked: RefCell<Vec<String>>,
}

impl Confirm {
    fn new(answer: bool) -> Self {
        Self {
            answer,
            asked: RefCell::default(),
        }
    }
}

impl Confirmer for Confirm {
    fn confirm(&self, prompt: &str) -> bool {
        self.asked.borrow_mut().push(prompt.to_string());
        self.answer
    }
}

fn opts() -> UpdateOptions {
    UpdateOptions {
        yes: true,
        ..UpdateOptions::default()
    }
}

fn do_run(
    current: &str,
    o: &UpdateOptions,
    c: &Client,
    i: &Inst,
    k: &Confirm,
) -> Result<Outcome, UpdateError> {
    run(current, TARGET, o, c, i, k)
}

#[test]
fn updates_to_latest_with_yes_and_skips_prompt() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(false));
    let out = do_run("1.2.0", &opts(), &c, &i, &k).unwrap();
    assert!(
        matches!(&out, Outcome::Updated { from, to } if from.tag() == "v1.2.0" && to.tag() == "v1.3.0")
    );
    assert_eq!(
        *i.installed.borrow(),
        vec![format!("branch-tally-v1.3.0-{TARGET}.tar.gz")]
    );
    assert!(k.asked.borrow().is_empty());
    assert_eq!(c.downloads(), 2);
    assert_eq!(out.message().lines().count(), 1);
    assert!(out.message().contains("1.2.0") && out.message().contains("1.3.0"));
}

#[test]
fn prompts_without_yes_and_installs_when_accepted() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions::default();
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(matches!(out, Outcome::Updated { .. }));
    assert_eq!(k.asked.borrow().len(), 1);
    assert_eq!(i.installed.borrow().len(), 1);
}

#[test]
fn declined_prompt_installs_and_downloads_nothing() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(false));
    let out = do_run("1.2.0", &UpdateOptions::default(), &c, &i, &k).unwrap();
    assert!(matches!(out, Outcome::Declined { .. }));
    assert!(i.installed.borrow().is_empty());
    assert_eq!(c.downloads(), 0);
    assert!(!out.is_refusal());
}

#[test]
fn up_to_date_does_nothing_without_force() {
    let (c, i, k) = (Client::new("v1.2.0"), Inst::default(), Confirm::new(true));
    let out = do_run("1.2.0", &opts(), &c, &i, &k).unwrap();
    assert!(matches!(out, Outcome::UpToDate { .. }));
    assert_eq!(c.downloads(), 0);
    assert!(i.installed.borrow().is_empty());
    assert!(k.asked.borrow().is_empty());
}

#[test]
fn latest_older_than_current_is_not_a_downgrade() {
    let (c, i, k) = (Client::new("v1.0.0"), Inst::default(), Confirm::new(true));
    let out = do_run("1.2.0", &opts(), &c, &i, &k).unwrap();
    assert!(matches!(out, Outcome::UpToDate { .. }));
    assert!(i.installed.borrow().is_empty());
}

#[test]
fn force_reinstalls_the_current_version() {
    let (c, i, k) = (Client::new("v1.2.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        force: true,
        yes: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(matches!(&out, Outcome::Updated { from, to } if from == to));
    assert_eq!(i.installed.borrow().len(), 1);
}

#[test]
fn check_reports_available_and_changes_nothing() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        check: true,
        yes: true,
        force: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(
        matches!(&out, Outcome::UpdateAvailable { current, available }
        if current.tag() == "v1.2.0" && available.tag() == "v1.3.0")
    );
    assert_eq!(c.downloads(), 0);
    assert!(i.installed.borrow().is_empty());
    assert!(k.asked.borrow().is_empty());
}

#[test]
fn check_reports_up_to_date() {
    let (c, i, k) = (Client::new("v1.2.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        check: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(matches!(out, Outcome::UpToDate { .. }));
    assert_eq!(c.downloads(), 0);
}

#[test]
fn check_does_not_need_a_writable_directory() {
    let c = Client::new("v1.3.0");
    let i = Inst {
        unwritable: Some("read-only".into()),
        ..Inst::default()
    };
    let o = UpdateOptions {
        check: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &Confirm::new(true)).unwrap();
    assert!(matches!(out, Outcome::UpdateAvailable { .. }));
}

#[test]
fn pinned_version_installs_that_release_without_asking_for_latest() {
    let (c, i, k) = (Client::new("v2.0.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        version: Some("1.3.0".into()),
        yes: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(matches!(&out, Outcome::Updated { to, .. } if to.tag() == "v1.3.0"));
    assert!(!c.log.borrow().iter().any(|l| l == "latest"));
}

#[test]
fn pinned_version_allows_downgrade() {
    let (c, i, k) = (Client::new("v2.0.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        version: Some("v1.0.0".into()),
        yes: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert!(
        matches!(&out, Outcome::Updated { from, to } if from.tag() == "v1.2.0" && to.tag() == "v1.0.0")
    );
    assert_eq!(
        *i.installed.borrow(),
        vec![format!("branch-tally-v1.0.0-{TARGET}.tar.gz")]
    );
}

#[test]
fn pinned_to_current_is_up_to_date_unless_forced() {
    let (c, i, k) = (Client::new("v2.0.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        version: Some("v1.2.0".into()),
        yes: true,
        ..UpdateOptions::default()
    };
    assert!(matches!(
        do_run("1.2.0", &o, &c, &i, &k).unwrap(),
        Outcome::UpToDate { .. }
    ));
    let o = UpdateOptions { force: true, ..o };
    assert!(matches!(
        do_run("1.2.0", &o, &c, &i, &k).unwrap(),
        Outcome::Updated { .. }
    ));
}

#[test]
fn invalid_pinned_version_is_a_typed_error() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        version: Some("banana".into()),
        yes: true,
        ..UpdateOptions::default()
    };
    let err = do_run("1.2.0", &o, &c, &i, &k).unwrap_err();
    assert!(matches!(err, UpdateError::InvalidVersion(_)));
    assert_eq!(c.calls(), 0);
}

#[test]
fn dev_build_is_refused_before_any_work() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    for o in [
        opts(),
        UpdateOptions {
            check: true,
            ..UpdateOptions::default()
        },
    ] {
        let out = do_run("0.0.0-dev", &o, &c, &i, &k).unwrap();
        assert_eq!(out, Outcome::Refused(Refusal::DevBuild));
        assert!(out.is_refusal());
        assert!(out.message().contains("install.sh"));
        assert!(out.message().contains("install.ps1"));
        assert_eq!(out.message().lines().count(), 1);
    }
    assert_eq!(c.calls(), 0);
    assert!(i.installed.borrow().is_empty());
}

#[test]
fn unwritable_directory_is_refused_before_downloading() {
    let c = Client::new("v1.3.0");
    let i = Inst {
        unwritable: Some("permission denied".into()),
        ..Inst::default()
    };
    let k = Confirm::new(true);
    let out = do_run("1.2.0", &opts(), &c, &i, &k).unwrap();
    assert_eq!(
        out,
        Outcome::Refused(Refusal::NotWritable("permission denied".into()))
    );
    assert!(out.message().contains("install.sh"));
    assert_eq!(c.calls(), 0);
    assert!(k.asked.borrow().is_empty());
}

#[test]
fn checksum_mismatch_installs_nothing() {
    let mut c = Client::new("v1.3.0");
    c.archive = b"tampered".to_vec();
    let i = Inst::default();
    let err = do_run("1.2.0", &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert!(matches!(err, UpdateError::ChecksumMismatch { .. }));
    assert!(i.installed.borrow().is_empty());
}

#[test]
fn archive_missing_from_sums_installs_nothing() {
    let mut c = Client::new("v1.3.0");
    c.sums_override = Some(format!("{}  some-other-file.tar.gz\n", hex(ARCHIVE)));
    let i = Inst::default();
    let err = do_run("1.2.0", &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert!(matches!(err, UpdateError::ChecksumMissing { .. }));
    assert!(i.installed.borrow().is_empty());
}

#[test]
fn missing_sums_file_installs_nothing_and_never_fetches_the_archive() {
    struct NoSums(Client);
    impl ReleaseClient for NoSums {
        fn latest_tag(&self) -> Result<String, FetchError> {
            self.0.latest_tag()
        }
        fn download(&self, tag: &str, asset: &str) -> Result<Vec<u8>, FetchError> {
            if asset == "SHA256SUMS" {
                self.0
                    .log
                    .borrow_mut()
                    .push(format!("download {tag} {asset}"));
                return Err(FetchError::NotFound);
            }
            self.0.download(tag, asset)
        }
    }
    let c = NoSums(Client::new("v1.3.0"));
    let i = Inst::default();
    let err = run("1.2.0", TARGET, &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert!(matches!(err, UpdateError::NotFound(_)));
    assert!(i.installed.borrow().is_empty());
    assert_eq!(c.0.downloads(), 1);
}

#[test]
fn unknown_pinned_tag_is_not_found() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        version: Some("v9.9.9".into()),
        yes: true,
        ..UpdateOptions::default()
    };
    let err = do_run("1.2.0", &o, &c, &i, &k).unwrap_err();
    assert!(matches!(err, UpdateError::NotFound(_)));
    assert!(err.to_string().contains("v9.9.9"));
    assert!(i.installed.borrow().is_empty());
}

#[test]
fn latest_not_found_and_network_errors_are_typed() {
    let mut c = Client::new("v1.3.0");
    c.latest = Err(FetchError::NotFound);
    let i = Inst::default();
    let err = do_run("1.2.0", &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert!(matches!(err, UpdateError::NotFound(_)));

    c.latest = Err(FetchError::Failed("timeout".into()));
    let err = do_run("1.2.0", &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert_eq!(err, UpdateError::Network("timeout".into()));
}

#[test]
fn garbage_latest_tag_is_a_typed_error() {
    let c = Client::new("not-a-version");
    let err = do_run("1.2.0", &opts(), &c, &Inst::default(), &Confirm::new(true)).unwrap_err();
    assert!(matches!(err, UpdateError::InvalidVersion(_)));
}

#[test]
fn network_error_messages_carry_no_urls() {
    let mut c = Client::new("v1.3.0");
    c.net_error = Some(FetchError::Failed(
        "error sending request for https://objects.example.com/x?token=SECRET123 (timed out)\nsecond line".into(),
    ));
    let err = do_run("1.2.0", &opts(), &c, &Inst::default(), &Confirm::new(true)).unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains("SECRET123"), "{msg}");
    assert!(!msg.contains("://"), "{msg}");
    assert!(msg.contains("timed out"), "{msg}");
    assert_eq!(msg.lines().count(), 1);
}

#[test]
fn install_failure_is_typed_and_scrubbed() {
    let c = Client::new("v1.3.0");
    let i = Inst {
        install_fails: true,
        ..Inst::default()
    };
    let err = do_run("1.2.0", &opts(), &c, &i, &Confirm::new(true)).unwrap_err();
    assert_eq!(err, UpdateError::Install("disk full".into()));
}

#[test]
fn unsupported_target_is_an_error_before_downloading() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let err = run("1.2.0", "riscv64-unknown-linux-gnu", &opts(), &c, &i, &k).unwrap_err();
    assert!(matches!(err, UpdateError::UnsupportedTarget(_)));
    assert_eq!(c.downloads(), 0);
}

#[test]
fn messages_are_single_lines() {
    let (c, i, k) = (Client::new("v1.3.0"), Inst::default(), Confirm::new(true));
    let o = UpdateOptions {
        check: true,
        ..UpdateOptions::default()
    };
    let out = do_run("1.2.0", &o, &c, &i, &k).unwrap();
    assert_eq!(out.message().lines().count(), 1);
    assert!(out.message().contains("1.3.0"));
}
