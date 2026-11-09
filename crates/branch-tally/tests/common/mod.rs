//! Shared harness: a throwaway home, a real git repository and a fixture scenario copied in under
//! that repository's real slug, as `scripts/make_goldens.py` does for the Python reference.
//!
//! The binary resolves `~/.claude/projects` from `HOME`/`USERPROFILE`, so a test points it at a
//! fixture by setting both. Nothing here touches the real home directory.

#![expect(clippy::expect_used, clippy::panic, reason = "test setup fails loudly")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_branch-tally");

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Where the shared fixtures live, relative to this crate.
pub(crate) fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude-code")
}

/// Claude Code's slug for a path: every non-alphanumeric becomes `-`.
pub(crate) fn slug_for(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=bt",
            "-c",
            "user.email=bt@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("these tests require `git` on PATH");
    assert!(out.status.success(), "git {args:?} failed");
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// A scratch directory removed on drop.
pub(crate) struct Scratch(pub PathBuf);

impl Scratch {
    pub(crate) fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("bt-e2e-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("create scratch");
        Self(canon(&dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A repository, a home with `.claude/projects`, and optionally a scenario copied into it.
pub(crate) struct World {
    _scratch: Scratch,
    pub(crate) home: PathBuf,
    pub(crate) repo: PathBuf,
    /// Lowercased directory name the binary reports as a session origin, back to the fixture's own.
    pub(crate) origins: BTreeMap<String, String>,
}

impl World {
    /// An empty home and a repository on `main` with one commit. No transcripts.
    pub(crate) fn empty() -> Self {
        let scratch = Scratch::new();
        let home = scratch.0.join("home");
        let repo = scratch.0.join("work").join("repo");
        fs::create_dir_all(home.join(".claude").join("projects")).expect("projects");
        fs::create_dir_all(&repo).expect("repo");
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        Self {
            _scratch: scratch,
            home,
            repo,
            origins: BTreeMap::new(),
        }
    }

    pub(crate) fn projects(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }

    /// Builds the scenario's worktrees and copies its directories in under this repo's real slug.
    pub(crate) fn scenario(name: &str) -> Self {
        let mut world = Self::empty();
        let dir = fixtures_root().join(name);
        let manifest = parse_json(
            &fs::read_to_string(dir.join("goldens").join("scenario.json")).expect("manifest"),
        );
        let repo_slug = manifest.get("repo_slug").as_str().to_owned();
        for wt in manifest.get("worktrees").as_arr() {
            let path = world
                .repo
                .join(".claude")
                .join("worktrees")
                .join(wt.get("name").as_str());
            git(
                &world.repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    wt.get("branch").as_str(),
                    &path.to_string_lossy(),
                ],
            );
        }
        let real = slug_for(&canon(&world.repo));
        let mut names: Vec<String> = fs::read_dir(&dir)
            .expect("scenario dir")
            .map(|e| e.expect("entry"))
            .filter(|e| e.file_type().expect("type").is_dir() && e.file_name() != "goldens")
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for original in names {
            let copied = if original
                .to_lowercase()
                .starts_with(&repo_slug.to_lowercase())
            {
                let suffix = &original[repo_slug.len()..];
                let exact = original[..repo_slug.len()] == repo_slug;
                format!(
                    "{}{suffix}",
                    if exact {
                        real.clone()
                    } else {
                        real.to_lowercase()
                    }
                )
            } else {
                original.clone()
            };
            copy_dir(&dir.join(&original), &world.projects().join(&copied));
            world.origins.insert(copied.to_lowercase(), original);
        }
        world
    }

    /// Runs the binary in the repository with this world's home.
    pub(crate) fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.repo, args)
    }

    pub(crate) fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("run branch-tally")
    }

    /// Replaces each non-main session origin with the fixture directory it came from.
    pub(crate) fn normalize(&self, payload: &mut Json) {
        let Json::Obj(top) = payload else { return };
        let Some(Json::Arr(sessions)) = top.get_mut("sessions") else {
            return;
        };
        for s in sessions {
            let Json::Obj(fields) = s else { continue };
            if let Some(Json::Str(origin)) = fields.get_mut("origin")
                && origin != "main"
            {
                *origin = self
                    .origins
                    .get(origin.as_str())
                    .unwrap_or_else(|| panic!("unexpected origin {origin}"))
                    .clone();
            }
        }
    }
}

/// The resolved path without a Windows verbatim (`\\?\`) prefix, which git rejects.
pub(crate) fn canon(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).expect("canonicalize");
    let text = resolved.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .map_or_else(|| resolved.clone(), PathBuf::from)
}

pub(crate) fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("stdout is UTF-8")
}

pub(crate) fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).expect("stderr is UTF-8")
}

/// A JSON value. The binary crate has no JSON dependency (a dev-dependency needs the Workspace
/// Config Steward), so tests carry this small parser.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(m) => m.get(key).unwrap_or_else(|| panic!("no key {key}")),
            _ => panic!("not an object"),
        }
    }
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("not a string"),
        }
    }
    pub(crate) fn as_arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => panic!("not an array"),
        }
    }
    pub(crate) fn entries(&self) -> &BTreeMap<String, Json> {
        match self {
            Json::Obj(m) => m,
            _ => panic!("not an object"),
        }
    }
}

pub(crate) fn parse_json(text: &str) -> Json {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.s.len(), "trailing data after JSON");
    v
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, b: u8) {
        self.ws();
        assert_eq!(self.s[self.i], b, "expected {:?} at {}", b as char, self.i);
        self.i += 1;
    }
    fn lit(&mut self, word: &str, v: Json) -> Json {
        assert!(self.s[self.i..].starts_with(word.as_bytes()), "bad literal");
        self.i += word.len();
        v
    }
    fn value(&mut self) -> Json {
        self.ws();
        match self.s[self.i] {
            b'n' => self.lit("null", Json::Null),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'"' => Json::Str(self.string()),
            b'[' => {
                self.i += 1;
                let mut out = Vec::new();
                self.ws();
                if self.s[self.i] == b']' {
                    self.i += 1;
                    return Json::Arr(out);
                }
                loop {
                    out.push(self.value());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    } else {
                        self.eat(b']');
                        return Json::Arr(out);
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut out = BTreeMap::new();
                self.ws();
                if self.s[self.i] == b'}' {
                    self.i += 1;
                    return Json::Obj(out);
                }
                loop {
                    self.ws();
                    let k = self.string();
                    self.eat(b':');
                    out.insert(k, self.value());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    } else {
                        self.eat(b'}');
                        return Json::Obj(out);
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                let n = std::str::from_utf8(&self.s[start..self.i]).expect("ascii");
                Json::Num(n.parse().unwrap_or_else(|_| panic!("bad number {n}")))
            }
        }
    }
    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = Vec::new();
        loop {
            let b = self.s[self.i];
            self.i += 1;
            match b {
                b'"' => return String::from_utf8(out).expect("utf-8"),
                b'\\' => {
                    let e = self.s[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex =
                                std::str::from_utf8(&self.s[self.i..self.i + 4]).expect("hex");
                            self.i += 4;
                            let c = char::from_u32(u32::from_str_radix(hex, 16).expect("hex"))
                                .expect("BMP scalar");
                            let mut buf = [0; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }
}
