//! Keyed-hash redaction of the committed ledger (decision 0005, #67).
//!
//! A [`Salt`] lives in `.tally/salt` in the main checkout; [`KeyedRedact`] replaces each
//! identifier with `HMAC-SHA256(salt, value)` as 32 lowercase hex characters and stamps the line
//! with the salt's fingerprint. The salt is committed on purpose, so it is not a secret.

use std::path::Path;

use ring::{digest, hmac};

use super::tally_dir::{hex, random_bytes};
use super::{LedgerEntry, LedgerError, Redact, StoredOrigin, TallyDir, line_text, parse_line};
use crate::model::{AgentId, RequestId, SelectedRecord, SessionId, SubagentTags, Workspace};

/// The salt's file name inside `.tally/`.
pub const SALT_FILE: &str = "salt";

/// A per-repository salt: 32 random bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Salt([u8; 32]);

impl std::fmt::Debug for Salt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Salt({})", self.fingerprint())
    }
}

impl Salt {
    /// A salt from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parses the file form: 64 lowercase hex characters, optionally followed by one line end.
    /// `None` for anything else.
    #[must_use]
    pub fn parse(text: &[u8]) -> Option<Self> {
        let text = text.strip_suffix(b"\n").unwrap_or(text);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        if text.len() != 64 {
            return None;
        }
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let mut bytes = [0_u8; 32];
        let (pairs, _) = text.as_chunks::<2>();
        for (out, [hi, lo]) in bytes.iter_mut().zip(pairs) {
            *out = nibble(*hi)? << 4 | nibble(*lo)?;
        }
        Some(Self(bytes))
    }

    /// A fresh salt from the system random number generator.
    ///
    /// # Errors
    /// [`std::io::Error`] when the generator fails.
    pub fn generate() -> std::io::Result<Self> {
        random_bytes().map(Self)
    }

    /// The file form, 64 lowercase hex characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    /// The first 16 hex characters of SHA-256 over the salt bytes: what a redacted line carries
    /// in `redaction`.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let sum = digest::digest(&digest::SHA256, &self.0);
        hex(&sum.as_ref()[..8])
    }

    /// `HMAC-SHA256(salt, value)`, lowercase hex truncated to 32 characters.
    #[must_use]
    pub fn hash(&self, value: &str) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.0);
        let tag = hmac::sign(&key, value.as_bytes());
        hex(&tag.as_ref()[..16])
    }

    /// A request id, hashed.
    #[must_use]
    pub fn request_id(&self, id: &RequestId) -> RequestId {
        RequestId::new(self.hash(id.as_str()))
    }

    /// A session id, hashed.
    #[must_use]
    pub fn session_id(&self, id: &SessionId) -> SessionId {
        SessionId::new(self.hash(id.as_str()))
    }

    /// An agent id, hashed.
    #[must_use]
    pub fn agent_id(&self, id: &AgentId) -> AgentId {
        AgentId::new(self.hash(id.as_str()))
    }

    /// Reads `.tally/salt`. `Ok(None)` when it does not exist. Never creates it.
    ///
    /// # Errors
    /// [`LedgerError::Refused`] for a link or non-regular file, [`LedgerError::MalformedSalt`]
    /// when the file is not a salt, [`LedgerError::Read`] otherwise.
    pub fn load(tally: &TallyDir) -> Result<Option<Self>, LedgerError> {
        let Some(bytes) = tally.read(SALT_FILE)? else {
            return Ok(None);
        };
        Self::parse(&bytes)
            .map(Some)
            .ok_or_else(|| LedgerError::MalformedSalt {
                path: tally.file_path(SALT_FILE),
            })
    }

    /// As [`Salt::load`], but re-reads a file that reads as malformed for a moment: the window in
    /// which a racing creator has made the file but not yet written its bytes. Never creates it.
    ///
    /// # Errors
    /// As [`Salt::load`], once the retry budget is spent.
    pub(crate) fn load_settled(tally: &TallyDir) -> Result<Option<Self>, LedgerError> {
        Self::load_settled_with(tally, &|| {})
    }

    /// [`Salt::load_settled`] with `on_malformed` run after each malformed read, before the
    /// retry: the test seam that orders a racing writer without relying on timing.
    ///
    /// # Errors
    /// As [`Salt::load_settled`].
    pub(crate) fn load_settled_with(
        tally: &TallyDir,
        on_malformed: &dyn Fn(),
    ) -> Result<Option<Self>, LedgerError> {
        let mut retries = 0_u32;
        loop {
            match Self::load(tally) {
                Err(LedgerError::MalformedSalt { .. }) if retries < SALT_READ_RETRIES => {
                    retries += 1;
                    on_malformed();
                    std::thread::sleep(std::time::Duration::from_millis(u64::from(retries)));
                }
                other => return other,
            }
        }
    }

    /// Reads `.tally/salt`, creating it exclusively when missing. When two runs race, exactly
    /// one creates it and the other reads the winner's. A malformed file is an error and is
    /// never replaced.
    ///
    /// # Errors
    /// As [`Salt::load`], plus [`LedgerError::Write`] when it cannot be created.
    pub fn load_or_create(tally: &TallyDir) -> Result<Self, LedgerError> {
        Self::settle(tally, &|| {
            Self::generate().map_err(|source| LedgerError::Write {
                path: tally.file_path(SALT_FILE),
                source,
            })
        })
    }

    /// The salt for the ledger in `target`: its own `salt` when it has one. Otherwise a copy of
    /// `main`'s salt when the main checkout's `.tally/` has one (so every worktree on a machine
    /// hashes alike), else a fresh one. The copy is an exclusive create, so a racer's salt wins
    /// and is reused. Nothing is ever written to `main`.
    ///
    /// # Errors
    /// As [`Salt::load_or_create`], and [`LedgerError::MalformedSalt`] when `main`'s salt file
    /// is damaged.
    pub fn resolve(target: &TallyDir, main: Option<&TallyDir>) -> Result<Self, LedgerError> {
        Self::settle(target, &|| {
            let Some(main) = main else {
                return Self::generate().map_err(|source| LedgerError::Write {
                    path: target.file_path(SALT_FILE),
                    source,
                });
            };
            match Self::load(main)? {
                Some(salt) => Ok(salt),
                None => Self::generate().map_err(|source| LedgerError::Write {
                    path: target.file_path(SALT_FILE),
                    source,
                }),
            }
        })
    }

    /// Loads the salt in `tally`, or creates one from `candidate`.
    fn settle(
        tally: &TallyDir,
        candidate: &dyn Fn() -> Result<Self, LedgerError>,
    ) -> Result<Self, LedgerError> {
        // A creator that lost the race may see the winner's file before its bytes land, which
        // reads as malformed for a moment. Retry briefly before believing it.
        let mut retries = 0_u32;
        loop {
            match Self::load(tally) {
                Ok(Some(salt)) => return Ok(salt),
                Ok(None) => {
                    let salt = candidate()?;
                    let text = format!("{}\n", salt.to_hex());
                    if tally.create_new(SALT_FILE, text.as_bytes())? {
                        return Ok(salt);
                    }
                }
                Err(LedgerError::MalformedSalt { .. }) if retries < SALT_READ_RETRIES => {
                    retries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(u64::from(retries)));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// How many times a salt that reads as malformed is re-read before it is reported.
const SALT_READ_RETRIES: u32 = 10;

/// Redacts ledger entries with a [`Salt`]: ids always, agent and skill names when `names` is on.
#[derive(Debug, Clone)]
pub struct KeyedRedact {
    salt: Salt,
    names: bool,
}

impl KeyedRedact {
    /// A redactor for `salt`. `names` also hashes agent and skill names.
    #[must_use]
    pub fn new(salt: Salt, names: bool) -> Self {
        Self { salt, names }
    }

    /// The salt.
    #[must_use]
    pub fn salt(&self) -> &Salt {
        &self.salt
    }

    /// A live record with the same fields hashed as a ledger entry's, so a union with a redacted
    /// ledger counts one call once.
    ///
    /// `main` is the main checkout, as for [`LedgerEntry::from_selected`]: a worktree origin is
    /// reduced to its name relative to it, hashed, and re-prefixed, exactly as a ledger entry's
    /// origin reads back, so the two compare equal. A worktree outside `main` stays unnamed.
    #[must_use]
    pub fn redact_selected(&self, record: SelectedRecord, main: Option<&Path>) -> SelectedRecord {
        let mut record = record;
        let r = &mut record.attributed.record;
        r.request_id = self.salt.request_id(&r.request_id);
        r.session_id = self.salt.session_id(&r.session_id);
        r.agent_id = r.agent_id.as_ref().map(|id| self.salt.agent_id(id));
        if self.names {
            r.attribution.skill = r.attribution.skill.as_deref().map(|n| self.salt.hash(n));
            r.attribution.agent = r.attribution.agent.as_deref().map(|n| self.salt.hash(n));
        }
        r.origin = r.origin.take().and_then(|o| {
            self.redact_origin(&StoredOrigin::from_origin(&o, main))
                .to_origin(main)
        });
        record.attributed.subagent = record.attributed.subagent.map(|t| self.tags(t));
        record.attributed.branch = self.redact_branch(&record.attributed.branch);
        record
    }

    /// A stored origin with its worktree name hashed (never optional: an `agent-<id>` name is an
    /// agent id). `Main` and an unnamed worktree are unchanged.
    #[must_use]
    pub fn redact_origin(&self, origin: &StoredOrigin) -> StoredOrigin {
        match origin {
            StoredOrigin::Worktree(Some(name)) => {
                StoredOrigin::Worktree(Some(self.salt.hash(name)))
            }
            other => other.clone(),
        }
    }

    /// A requested branch name as the ledger would store it: `worktree-agent-<id>` becomes
    /// `worktree-agent-<hash(id)>` and every other name is unchanged. It always hashes: whether a
    /// stored line was already hashed is the line's `redaction` stamp's business, not the id's
    /// shape.
    #[must_use]
    pub fn redact_branch(&self, branch: &str) -> String {
        match branch.strip_prefix(AGENT_BRANCH_PREFIX) {
            Some(id) if !id.is_empty() => {
                format!("{AGENT_BRANCH_PREFIX}{}", self.salt.hash(id))
            }
            _ => branch.to_owned(),
        }
    }

    /// A line stamped under another salt, with hashed ids and a raw agent branch. Repairing it
    /// would relabel old-salt hashes as this salt's, so it is left as it is.
    fn is_foreign_repair(&self, entry: &LedgerEntry) -> bool {
        entry
            .redaction
            .as_deref()
            .is_some_and(|stamp| stamp != self.salt.fingerprint())
            && ids_hashed(entry)
    }

    /// The tags with their agent id hashed. The `subagent` label is the agent id when no name was
    /// stamped, so that case is an id and is hashed regardless of `names`.
    fn tags(&self, tags: SubagentTags) -> SubagentTags {
        let label_is_id = tags.subagent == tags.agent_id.as_str();
        SubagentTags {
            subagent: if label_is_id || self.names {
                self.salt.hash(&tags.subagent)
            } else {
                tags.subagent
            },
            agent_id: self.salt.agent_id(&tags.agent_id),
            workspace: match tags.workspace {
                Workspace::Worktree(name) => Workspace::worktree(self.salt.hash(&name)),
                other => other,
            },
        }
    }
}

impl Redact for KeyedRedact {
    fn redact(&self, entry: LedgerEntry) -> LedgerEntry {
        if is_hashed_line(&entry) {
            return entry;
        }
        if self.is_foreign_repair(&entry) {
            return entry;
        }
        let mut entry = entry;
        let stamped = entry.redaction.is_some();
        // A stamped line's branches are hashed only where they are visibly raw, so a repair
        // never hashes a hash.
        let branch = |b: &str| {
            if !stamped || branch_is_raw(b) {
                self.redact_branch(b)
            } else {
                b.to_owned()
            }
        };
        entry.branch = branch(&entry.branch);
        entry.recorded_for = branch(&entry.recorded_for);
        if stamped && ids_hashed(&entry) {
            // Written by an earlier build: ids hashed, only a branch was left raw. Repaired
            // above; the ids must not be hashed again.
            entry.redaction = Some(self.salt.fingerprint());
            return entry;
        }
        entry.request_id = self.salt.request_id(&entry.request_id);
        entry.session_id = self.salt.session_id(&entry.session_id);
        entry.agent_id = entry.agent_id.as_ref().map(|id| self.salt.agent_id(id));
        if self.names {
            entry.skill = entry.skill.as_deref().map(|n| self.salt.hash(n));
            entry.agent = entry.agent.as_deref().map(|n| self.salt.hash(n));
        }
        entry.origin = entry.origin.as_ref().map(|o| self.redact_origin(o));
        entry.subagent = entry.subagent.map(|t| self.tags(t));
        entry.redaction = Some(self.salt.fingerprint());
        entry
    }
}

/// The branch Claude Code gives an agent's worktree, followed by the agent id.
const AGENT_BRANCH_PREFIX: &str = "worktree-agent-";

/// Whether `branch` is an agent worktree's branch, `worktree-agent-<id>`: the one branch name a
/// redacted line stores hashed.
pub(crate) fn is_agent_branch(branch: &str) -> bool {
    branch
        .strip_prefix(AGENT_BRANCH_PREFIX)
        .is_some_and(|id| !id.is_empty())
}

/// Whether `text` is what [`Salt::hash`] writes: 32 lowercase hex characters.
fn is_hash(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether a line is genuinely redacted: it carries the stamp AND its request, session and agent
/// ids are hashes. A stamp over raw ids (a forgery or a bug) is not believed.
fn is_hashed_line(entry: &LedgerEntry) -> bool {
    entry.redaction.is_some()
        && ids_hashed(entry)
        && !branch_is_raw(&entry.branch)
        && !branch_is_raw(&entry.recorded_for)
}

/// Whether the request, session and agent ids are all hashes.
fn ids_hashed(entry: &LedgerEntry) -> bool {
    is_hash(entry.request_id.as_str())
        && is_hash(entry.session_id.as_str())
        && entry
            .agent_id
            .as_ref()
            .is_none_or(|id| is_hash(id.as_str()))
}

/// Whether a stored branch is visibly a raw agent branch: `worktree-agent-<x>` with `<x>` not a
/// hash. The shape is only used to detect a raw value written by an earlier build; it is never a
/// reason to skip hashing one.
fn branch_is_raw(branch: &str) -> bool {
    branch
        .strip_prefix(AGENT_BRANCH_PREFIX)
        .is_some_and(|id| !id.is_empty() && !is_hash(id))
}

/// What `ledger redact` did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RedactOutcome {
    /// Non-blank lines seen.
    pub lines: u64,
    /// Unredacted lines rewritten.
    pub rewritten: u64,
    /// Lines that already carry `redaction`, left byte-identical.
    pub already_redacted: u64,
    /// Lines that do not parse (or whose `ts` is not RFC 3339), left byte-identical.
    pub unparseable: u64,
    /// Lines that carried a `redaction` stamp but raw ids, so the stamp was not believed: they
    /// were hashed and stamped again.
    pub restamped: u64,
    /// Lines with hashed ids and a raw agent branch, stamped under a salt other than the current
    /// one. Repairing them would relabel old-salt hashes as the current salt, so they are left
    /// byte-identical and counted here.
    pub foreign_salt_unrepaired: u64,
    /// `.tally/` does not exist: nothing was read or written.
    pub tally_absent: bool,
}

/// Rewrites the unredacted lines of the explicit ledger at `path`.
///
/// # Errors
/// [`LedgerError`] when the ledger cannot be read or replaced; the original is then untouched.
pub fn redact_ledger(path: &Path, redactor: &KeyedRedact) -> Result<RedactOutcome, LedgerError> {
    redact_ledger_with(path, redactor, &|| {})
}

/// `redact_ledger` with a seam that runs after the rewrite is written to its temporary file and
/// before the original is checked and replaced.
pub(super) fn redact_ledger_with(
    path: &Path,
    redactor: &KeyedRedact,
    between: &dyn Fn(),
) -> Result<RedactOutcome, LedgerError> {
    let real = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RedactOutcome::default()),
        Err(source) => {
            return Err(LedgerError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let existing = std::fs::read(&real).map_err(|source| LedgerError::Read {
        path: path.to_owned(),
        source,
    })?;
    let (bytes, outcome) = rewrite(&existing, redactor);
    if outcome.rewritten + outcome.restamped > 0 {
        replace_file(&real, &bytes, &existing, between).map_err(|source| {
            if source.kind() == std::io::ErrorKind::InvalidData {
                LedgerError::ChangedDuringRewrite {
                    path: path.to_owned(),
                }
            } else {
                LedgerError::Write {
                    path: path.to_owned(),
                    source,
                }
            }
        })?;
    }
    Ok(outcome)
}

/// Writes `bytes` beside `target` under a random name, created exclusively, checks `target`
/// still holds `expected` (an `InvalidData` error otherwise), then renames over it. The temporary
/// file is removed on failure. A write landing between the check and the rename is not detected.
fn replace_file(
    target: &Path,
    bytes: &[u8],
    expected: &[u8],
    between: &dyn Fn(),
) -> std::io::Result<()> {
    use std::io::Write;

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ledger");
    let temp = target.with_file_name(format!("{name}.{}.tmp", hex(&random_bytes()?[..8])));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let done = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            between();
            if std::fs::read(target)? == expected {
                std::fs::rename(&temp, target)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "changed while rewriting",
                ))
            }
        });
    if done.is_err() {
        drop(file);
        let _ = std::fs::remove_file(&temp);
    }
    done
}

/// Redacts the lines of `existing` that parse and are unredacted; every other byte is kept.
fn rewrite(existing: &[u8], redactor: &KeyedRedact) -> (Vec<u8>, RedactOutcome) {
    let mut outcome = RedactOutcome::default();
    let mut out: Vec<u8> = Vec::with_capacity(existing.len());
    for (i, piece) in existing.split(|b| *b == b'\n').enumerate() {
        if i > 0 {
            out.push(b'\n');
        }
        let raw = piece.strip_suffix(b"\r").unwrap_or(piece);
        if raw.iter().all(u8::is_ascii_whitespace) {
            out.extend_from_slice(piece);
            continue;
        }
        outcome.lines += 1;
        match parse_line(raw) {
            Ok((entry, false)) if entry.redaction.is_none() => {
                outcome.rewritten += 1;
                out.extend_from_slice(line_text(&redactor.redact(entry)).as_bytes());
            }
            Ok((entry, false)) if !is_hashed_line(&entry) && redactor.is_foreign_repair(&entry) => {
                outcome.foreign_salt_unrepaired += 1;
                out.extend_from_slice(piece);
            }
            Ok((entry, false)) if !is_hashed_line(&entry) => {
                outcome.restamped += 1;
                out.extend_from_slice(line_text(&redactor.redact(entry)).as_bytes());
            }
            Ok((_, false)) => {
                outcome.already_redacted += 1;
                out.extend_from_slice(piece);
            }
            _ => {
                outcome.unparseable += 1;
                out.extend_from_slice(piece);
            }
        }
    }
    (out, outcome)
}

/// Rewrites the unredacted lines of `.tally/ledger.jsonl` under the checkout `root`, through
/// [`TallyDir`].
///
/// # Errors
/// As [`redact_ledger`], plus [`LedgerError::Refused`].
pub fn redact_ledger_default(
    root: &Path,
    redactor: &KeyedRedact,
) -> Result<RedactOutcome, LedgerError> {
    redact_ledger_default_with(root, redactor, &|| {})
}

/// `redact_ledger_default` with the same seam as [`redact_ledger_with`].
pub(super) fn redact_ledger_default_with(
    root: &Path,
    redactor: &KeyedRedact,
    between: &dyn Fn(),
) -> Result<RedactOutcome, LedgerError> {
    let Some(tally) = TallyDir::open(root)? else {
        return Ok(RedactOutcome {
            tally_absent: true,
            ..RedactOutcome::default()
        });
    };
    let Some(existing) = tally.read(super::LEDGER_FILE)? else {
        return Ok(RedactOutcome::default());
    };
    let (bytes, outcome) = rewrite(&existing, redactor);
    if outcome.rewritten + outcome.restamped > 0 {
        tally.replace_with(super::LEDGER_FILE, &bytes, Some(&existing), between)?;
    }
    Ok(outcome)
}
