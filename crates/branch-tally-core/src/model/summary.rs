//! The shape `summary` produces and `render` consumes. Types and accessors only.
//!
//! Tables are `Vec`s in **first-seen order**, not maps. The reference iterates Python dicts
//! (insertion order) and then does stable sorts (`-output`, `-tokens`, `first`), so ties keep
//! first-seen order. A `Vec` lets render reproduce that exactly; a `HashMap` or `BTreeMap` would
//! not. The ordering contract is unchanged: the summary stage pushes in first-seen order.
//!
//! Everything `render` and `as_json` would otherwise compute is precomputed here (the Total row,
//! the footnote counts), so a renderer never re-derives totals. Cost is not stored: `pricing`
//! produces [`super::Costs`] from `by_model`. The requested branch and the mode live on
//! [`Summary`] rather than being render arguments, because both `render` and `as_json` print
//! them and they describe how the numbers were selected.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{SessionId, SubagentSummary, saturating_add_flag};

/// Summed token counts (`Bucket` in `reference/token_report.py`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    /// Uncached input tokens.
    pub input: u64,
    /// 5-minute cache writes (`w5`).
    pub cache_write_5m: u64,
    /// 1-hour cache writes (`w1h`).
    pub cache_write_1h: u64,
    /// Cache reads (`read`).
    pub cache_read: u64,
    /// Output tokens, thinking included.
    pub output: u64,
    /// Thinking share of `output` (`think`); not added to totals.
    pub thinking: u64,
    /// Number of calls summed.
    pub calls: u64,
}

impl Bucket {
    /// Input + both cache writes + cache read + output; the reference's `tok` in the
    /// attribution table. Thinking is excluded: it is already inside output.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.checked_tokens().unwrap_or(u64::MAX)
    }

    /// [`Bucket::tokens`], or `None` when the sum overflows `u64`.
    #[must_use]
    pub fn checked_tokens(&self) -> Option<u64> {
        [
            self.input,
            self.cache_write_5m,
            self.cache_write_1h,
            self.cache_read,
            self.output,
        ]
        .into_iter()
        .try_fold(0u64, u64::checked_add)
    }

    /// Adds `other` into `self` field by field, saturating at `u64::MAX`. Returns `true` when any
    /// field saturated, so the caller can mark its total unreliable ([`Footnote::totals_overflowed`]).
    pub fn saturating_add_assign(&mut self, other: &Bucket) -> bool {
        let mut over = false;
        for (a, b) in [
            (&mut self.input, other.input),
            (&mut self.cache_write_5m, other.cache_write_5m),
            (&mut self.cache_write_1h, other.cache_write_1h),
            (&mut self.cache_read, other.cache_read),
            (&mut self.output, other.output),
            (&mut self.thinking, other.thinking),
            (&mut self.calls, other.calls),
        ] {
            over |= saturating_add_flag(a, b);
        }
        over
    }
}

/// One session's row (`Session` in the reference). `first`/`last` are raw ISO strings compared
/// as strings, and `""` when a record had no timestamp, exactly as the reference does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRow {
    /// Earliest timestamp seen.
    pub first: String,
    /// Latest timestamp seen.
    pub last: String,
    /// Transcript root of the session's first record: `"main"` or a worktree slug, as text.
    pub origin: String,
    /// Total tokens (the reference's `Session.tokens`).
    pub tokens: u64,
    /// Number of calls.
    pub calls: u64,
    /// Names of subagents seen (the `agent:` suffixes). Sorted by construction.
    pub agents: BTreeSet<String>,
}

impl SessionRow {
    /// A row for the session's first record, with no tokens, calls or agents yet.
    #[must_use]
    pub fn new(first: impl Into<String>, origin: impl Into<String>) -> Self {
        let first = first.into();
        Self {
            last: first.clone(),
            first,
            origin: origin.into(),
            tokens: 0,
            calls: 0,
            agents: BTreeSet::new(),
        }
    }

    /// The reference's `Session.who`: agents joined by `", "`, or an em dash when none.
    #[must_use]
    pub fn who(&self) -> String {
        if self.agents.is_empty() {
            "\u{2014}".to_owned()
        } else {
            self.agents
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

/// How records were selected for the branch (`--strict` in the reference).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Naive branch filter: no session claim, no worktree recovery.
    Strict,
    /// Session claim and worktree recovery (the default).
    #[default]
    Claim,
    /// Per-session branch timeline: records are placed by `(ts, gitBranch)` on the work branch
    /// active at that time. A heuristic; the report must say so.
    Timeline,
}

impl Mode {
    /// The `mode` string `as_json` prints: `"strict"`, `"claim"` or `"timeline"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Claim => "claim",
            Self::Timeline => "timeline",
        }
    }
}

/// Calls and tokens attributed by one heuristic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicCount {
    /// Number of calls.
    pub calls: u64,
    /// Total tokens, counted as [`Bucket::tokens`] does (thinking excluded).
    pub tokens: u64,
}

impl HeuristicCount {
    /// Counts one call of `tokens` tokens.
    pub fn add(&mut self, tokens: u64) {
        self.add_checked(tokens);
    }

    /// [`HeuristicCount::add`] that saturates at `u64::MAX` and returns `true` when either
    /// field saturated.
    pub fn add_checked(&mut self, tokens: u64) -> bool {
        let calls = saturating_add_flag(&mut self.calls, 1);
        let toks = saturating_add_flag(&mut self.tokens, tokens);
        calls | toks
    }
}

/// Calls and tokens per attribution heuristic, over the records the report kept. Filled by the
/// summary stage, printed by render. A record counts under exactly one field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicBreakdown {
    /// Kept only because its session was claimed ([`super::Selection::SessionClaim`]).
    pub session_claim: HeuristicCount,
    /// [`super::BranchBasis::WorktreeRecovered`].
    pub worktree_recovery: HeuristicCount,
    /// [`super::BranchBasis::OriginRecovered`].
    pub origin_recovery: HeuristicCount,
    /// [`super::BranchBasis::DispatcherRolledUp`].
    pub dispatcher_roll_up: HeuristicCount,
    /// [`super::BranchBasis::TimelineForward`].
    pub timeline_forward: HeuristicCount,
    /// [`super::BranchBasis::TimelineCarried`].
    pub timeline_carried: HeuristicCount,
    /// [`super::BranchBasis::Captured`]: placed by a captured hook event.
    #[serde(default)]
    pub captured: HeuristicCount,
    /// [`super::BranchBasis::TimelineAmbiguous`] plus [`super::BranchBasis::SubagentUnlinked`]:
    /// records that could not be placed with certainty. Any call here makes the total a floor.
    pub ambiguous: HeuristicCount,
}

impl HeuristicBreakdown {
    /// `true` when any record could not be attributed with certainty, so the report must say
    /// "at least" and show the ambiguous token count.
    #[must_use]
    pub fn any_ambiguous(&self) -> bool {
        self.ambiguous.calls > 0
    }
}

/// Where the default branch a timeline report excluded came from. Anything but
/// [`Self::OriginHead`] is a fallback, and the report says so (Governance II).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultBranchSource {
    /// `refs/remotes/origin/HEAD`: what the remote says.
    OriginHead,
    /// `git config init.defaultBranch`, used because `origin/HEAD` was unresolvable.
    InitDefaultBranch,
    /// Neither was available: `main` was assumed.
    AssumedMain,
}

impl DefaultBranchSource {
    /// Text for the report: `origin/HEAD`, `init.defaultBranch` or `assumed`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OriginHead => "origin/HEAD",
            Self::InitDefaultBranch => "init.defaultBranch",
            Self::AssumedMain => "assumed",
        }
    }
}

/// Which branches timeline mode treated as work branches. Recorded on the summary so the report
/// can state it. Not transcript content: a branch name and a user-supplied pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkBranchRule {
    /// Every branch except the default branch and `worktree-agent-*`.
    ExcludesDefault {
        /// The default branch that was excluded.
        branch: String,
        /// Where `branch` came from.
        source: DefaultBranchSource,
    },
    /// A custom `--work-branch` pattern was used (its text).
    Custom {
        /// The pattern as the user gave it.
        pattern: String,
    },
}

/// The counts `render`'s footnote prints, precomputed by the summary stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Footnote {
    /// Calls and tokens per attribution heuristic. Absent in older serialized data.
    #[serde(default)]
    pub heuristics: HeuristicBreakdown,
    /// Calls with `is_sidechain` set (`sub_calls`).
    pub subagent_calls: u64,
    /// Tokens of records whose assigned branch is not the requested one (`off_branch`): the
    /// "recovered from pre-branch/worktree checkouts" figure.
    pub off_branch_tokens: u64,
    /// Sessions whose origin is not `"main"` (`worktree_sessions`).
    pub worktree_sessions: u64,
    /// `true` when a token or call total saturated at `u64::MAX`: the totals are then a floor
    /// and unreliable (a fabricated or corrupt ledger line can carry absurd counts). Set by the
    /// summary stage from the `bool` the saturating helpers return. Omitted from JSON when
    /// `false`; absent in older serialized data.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub totals_overflowed: bool,
    /// Set when the total may count some calls twice (decision 0005, "Mixing is counted and
    /// noted"): an overcount, not a floor, so it is never framed as "at least". `None` in every
    /// path that cannot double count. Carries counts only, no ids, paths or salts. Omitted from
    /// JSON when `None`; absent in older serialized data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_count: Option<DoubleCountRisk>,
}

/// Why a total may count some calls twice. At least one of the parts is set when built through
/// [`DoubleCountRisk::new`]; both can be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoubleCountRisk {
    /// The ledger mixes unredacted and redacted lines, or several salts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixed_ledger: Option<MixedLedger>,
    /// Live calls could not be matched to the ledger's redacted lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_match: Option<LiveMatchFailure>,
}

impl DoubleCountRisk {
    /// Builds the risk, or `None` when neither cause is present, so `Some` always means a real
    /// risk.
    #[must_use]
    pub fn new(
        mixed_ledger: Option<MixedLedger>,
        live_match: Option<LiveMatchFailure>,
    ) -> Option<Self> {
        if mixed_ledger.is_none() && live_match.is_none() {
            return None;
        }
        Some(Self {
            mixed_ledger,
            live_match,
        })
    }
}

/// Line counts of a mixed ledger. Counts only, never ids or salts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedLedger {
    /// Lines carrying plain identifiers.
    pub unredacted_lines: u64,
    /// Lines carrying keyed hashes.
    pub redacted_lines: u64,
    /// Distinct salt fingerprints across the redacted lines.
    pub distinct_salts: u64,
}

/// Why live calls could not be matched to redacted ledger lines. Mirrors the four failure
/// variants of `report::LiveRedactionMatch`, which this module cannot depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LiveMatchFailure {
    /// The ledger has redacted lines and there is no local salt.
    NoSalt,
    /// A local salt exists but could not be read.
    SaltUnreadable,
    /// The local salt's fingerprint is on none of the redacted lines.
    FingerprintMismatch,
    /// The local salt matches some redacted lines; others cannot be matched.
    PartiallyMatched,
}

/// Calls that no branch's report places (#60): a record whose branch is `(none)` after
/// attribution and that no branch's selection would keep, under the report's mode. In claim mode
/// a session claim places a `(none)` record on the session's only work-shaped branch, so it is not
/// counted; in strict and timeline modes there is no claim, so every `(none)` record is.
/// Subagent records with no dispatch link are excluded (they are in
/// [`SubagentSummary`]'s `unattributed_*`). Counted by `summary::count_unstamped` over the
/// **full attributed set before selection**, so it is the same for every branch's report.
///
/// It can overlap the ambiguous heuristic: a timeline-ambiguous record that kept `(none)` is
/// counted here, and also in `footnote.heuristics.ambiguous` when its session has a record
/// selected for the branch. That count is session-scoped, so it can't be excluded here without
/// making this one differ by branch.
///
/// Present in live and ledger+live reports; a ledger-only report cannot know it and leaves it
/// zero (not "none exist"). Ledger entries are placed by definition (selected for their
/// `recorded_for` branch), so only live records are counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unstamped {
    /// Calls with no branch stamp that were not attributed to any branch.
    pub calls: u64,
    /// Tokens of those calls (same definition as [`Bucket::tokens`]).
    pub tokens: u64,
}

/// Unresolved pending ledger lines (ADR 0006): usage recorded on the default branch and not yet
/// resolved to a branch. Deduplicated by request id and kept outside the totals, so any nonzero
/// count makes the totals beside it a floor. Absent in older serialized data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// Unresolved pending ledger lines.
    pub lines: u64,
    /// Calls on those lines.
    pub calls: u64,
    /// Tokens of those calls (same definition as [`Bucket::tokens`]).
    pub tokens: u64,
}

impl Pending {
    /// Whether there is pending usage outside the totals, which makes them a floor.
    #[must_use]
    pub fn any(&self) -> bool {
        self.lines > 0
    }
}

/// The three tables `summarize` returns, each in first-seen order, plus what render derives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// The requested branch name.
    pub branch: String,
    /// How records were selected.
    pub mode: Mode,
    /// Sum of `by_model`: the Total row (`total` in `render`).
    pub total: Bucket,
    /// The footnote counts.
    pub footnote: Footnote,
    /// Per model id (`by_model`).
    pub by_model: Vec<(String, Bucket)>,
    /// Per attribution label such as `main`, `agent:x` or a skill name (`by_attr`).
    pub by_attribution: Vec<(String, Bucket)>,
    /// Per session id (`sessions`).
    pub sessions: Vec<(SessionId, SessionRow)>,
    /// The dispatched-subagent breakdown (workspace and subagent tags). Empty in strict mode.
    /// Absent in older serialized data.
    #[serde(default)]
    pub subagents: SubagentSummary,
    /// Which branches timeline mode treated as work branches. `None` outside timeline mode.
    /// Absent in older serialized data.
    #[serde(default)]
    pub work_branch: Option<WorkBranchRule>,
    /// Calls with no branch stamp that no branch was assigned (#60). Repository-wide. Zero when
    /// there are none, and also when the count is unknown (ledger-only). Absent in older
    /// serialized data.
    #[serde(default)]
    pub unstamped: Unstamped,
    /// Unresolved pending ledger lines (ADR 0006), outside the totals. Absent in older
    /// serialized data.
    #[serde(default)]
    pub pending: Pending,
    /// `true` when `redact` replaced identifying detail. Absent in older serialized data.
    #[serde(default)]
    pub redacted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_without_pending_deserializes_to_zero_and_round_trips_nonzero() {
        let mut v = serde_json::to_value(Summary::default()).unwrap();
        v.as_object_mut().unwrap().remove("pending");
        let old: Summary = serde_json::from_value(v).unwrap();
        assert_eq!(old.pending, Pending::default());

        let s = Summary {
            pending: Pending {
                lines: 2,
                calls: 5,
                tokens: 700,
            },
            ..Summary::default()
        };
        let back: Summary = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(
            back.pending,
            Pending {
                lines: 2,
                calls: 5,
                tokens: 700
            }
        );
    }

    #[test]
    fn bucket_tokens_exclude_thinking_and_calls() {
        let b = Bucket {
            input: 1,
            cache_write_5m: 2,
            cache_write_1h: 3,
            cache_read: 4,
            output: 5,
            thinking: 100,
            calls: 100,
        };
        assert_eq!(b.tokens(), 15);
    }

    fn max_bucket() -> Bucket {
        Bucket {
            input: u64::MAX,
            cache_write_5m: u64::MAX,
            cache_write_1h: u64::MAX,
            cache_read: u64::MAX,
            output: u64::MAX,
            thinking: u64::MAX,
            calls: u64::MAX,
        }
    }

    #[test]
    fn bucket_tokens_saturate_instead_of_wrapping() {
        assert_eq!(max_bucket().tokens(), u64::MAX);
        assert_eq!(max_bucket().checked_tokens(), None);
        let small = Bucket {
            input: 1,
            output: 2,
            ..Bucket::default()
        };
        assert_eq!(small.checked_tokens(), Some(3));
    }

    #[test]
    fn adding_buckets_near_max_saturates_and_reports_overflow() {
        let mut a = Bucket {
            input: u64::MAX - 1,
            calls: 1,
            ..Bucket::default()
        };
        let b = Bucket {
            input: 5,
            output: 7,
            calls: 2,
            ..Bucket::default()
        };
        assert!(a.saturating_add_assign(&b));
        assert_eq!((a.input, a.output, a.calls), (u64::MAX, 7, 3));
    }

    #[test]
    fn adding_small_buckets_is_exact_and_reports_no_overflow() {
        let mut a = Bucket {
            input: 1,
            calls: 1,
            thinking: 4,
            ..Bucket::default()
        };
        let b = Bucket {
            input: 2,
            calls: 1,
            thinking: 5,
            ..Bucket::default()
        };
        assert!(!a.saturating_add_assign(&b));
        assert_eq!((a.input, a.calls, a.thinking), (3, 2, 9));
    }

    #[test]
    fn heuristic_count_saturates_and_reports_overflow() {
        let mut c = HeuristicCount {
            calls: u64::MAX,
            tokens: 1,
        };
        assert!(c.add_checked(2));
        assert_eq!((c.calls, c.tokens), (u64::MAX, 3));
        let mut d = HeuristicCount {
            calls: 0,
            tokens: u64::MAX,
        };
        assert!(d.add_checked(1));
        assert_eq!(d.tokens, u64::MAX);
    }

    #[test]
    fn overflowed_is_omitted_from_json_unless_set_and_defaults_false() {
        let mut s = Summary::default();
        assert!(!serde_json::to_string(&s).unwrap().contains("overflowed"));
        s.footnote.totals_overflowed = true;
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"totals_overflowed\":true"));
        let back: Summary = serde_json::from_str(&json).unwrap();
        assert!(back.footnote.totals_overflowed);
    }

    #[test]
    fn who_is_sorted_and_joined_or_em_dash() {
        let mut s = SessionRow::new("2026-01-01T00:00:00Z", "main");
        assert_eq!(s.who(), "\u{2014}");
        s.agents.insert("zed".to_owned());
        s.agents.insert("alpha".to_owned());
        assert_eq!(s.who(), "alpha, zed");
    }

    #[test]
    fn new_row_starts_with_last_equal_first_and_nothing_counted() {
        let s = SessionRow::new("t", "main");
        assert_eq!((s.first.as_str(), s.last.as_str()), ("t", "t"));
        assert_eq!((s.tokens, s.calls), (0, 0));
    }

    #[test]
    fn mode_names_match_as_json_and_default_is_claim() {
        assert_eq!(Mode::Strict.as_str(), "strict");
        assert_eq!(Mode::Claim.as_str(), "claim");
        assert_eq!(Mode::default(), Mode::Claim);
        assert_eq!(serde_json::to_string(&Mode::Strict).unwrap(), "\"strict\"");
    }

    #[test]
    fn timeline_mode_has_its_name_and_claim_stays_default() {
        assert_eq!(Mode::Timeline.as_str(), "timeline");
        assert_eq!(Mode::default(), Mode::Claim);
        assert_eq!(
            serde_json::to_string(&Mode::Timeline).unwrap(),
            "\"timeline\""
        );
    }

    #[test]
    fn heuristic_count_adds_calls_and_tokens() {
        let mut c = HeuristicCount::default();
        c.add(10);
        c.add(5);
        assert_eq!((c.calls, c.tokens), (2, 15));
    }

    #[test]
    fn any_ambiguous_follows_the_ambiguous_calls_only() {
        let mut b = HeuristicBreakdown::default();
        assert!(!b.any_ambiguous());
        b.timeline_carried.add(7);
        assert!(!b.any_ambiguous());
        b.ambiguous.add(0);
        assert!(b.any_ambiguous());
    }

    #[test]
    fn double_count_defaults_to_none_and_old_json_loads() {
        assert_eq!(Footnote::default().double_count, None);
        let json = r#"{"subagent_calls":1,"off_branch_tokens":2,"worktree_sessions":3}"#;
        let f: Footnote = serde_json::from_str(json).unwrap();
        assert_eq!(f.double_count, None);
        assert!(!serde_json::to_string(&f).unwrap().contains("double_count"));
    }

    #[test]
    fn double_count_new_is_none_without_a_cause() {
        assert_eq!(DoubleCountRisk::new(None, None), None);
    }

    #[test]
    fn double_count_new_keeps_both_causes_and_round_trips() {
        let m = MixedLedger {
            unredacted_lines: 2,
            redacted_lines: 5,
            distinct_salts: 2,
        };
        let r = DoubleCountRisk::new(Some(m), Some(LiveMatchFailure::PartiallyMatched)).unwrap();
        assert_eq!(r.mixed_ledger, Some(m));
        assert_eq!(r.live_match, Some(LiveMatchFailure::PartiallyMatched));
        let only = DoubleCountRisk::new(None, Some(LiveMatchFailure::NoSalt)).unwrap();
        assert_eq!(only.mixed_ledger, None);
        let f = Footnote {
            double_count: Some(r),
            ..Footnote::default()
        };
        let back: Footnote = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn footnote_without_heuristics_still_loads() {
        let json = r#"{"subagent_calls":1,"off_branch_tokens":2,"worktree_sessions":3}"#;
        let f: Footnote = serde_json::from_str(json).unwrap();
        assert_eq!(f.heuristics, HeuristicBreakdown::default());
        assert_eq!(f.worktree_sessions, 3);
        let back: Footnote = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn summary_without_redacted_loads_as_not_redacted() {
        let json = r#"{"branch":"b","mode":"claim","total":{"input":0,"cache_write_5m":0,"cache_write_1h":0,"cache_read":0,"output":0,"thinking":0,"calls":0},"footnote":{"subagent_calls":0,"off_branch_tokens":0,"worktree_sessions":0},"by_model":[],"by_attribution":[],"sessions":[]}"#;
        let s: Summary = serde_json::from_str(json).unwrap();
        assert!(!s.redacted);
        assert!(!Summary::default().redacted);
    }

    #[test]
    fn summary_round_trips_and_keeps_first_seen_order() {
        let mut sum = Summary::default();
        sum.by_model.push(("z".to_owned(), Bucket::default()));
        sum.by_model.push(("a".to_owned(), Bucket::default()));
        let back: Summary = serde_json::from_str(&serde_json::to_string(&sum).unwrap()).unwrap();
        assert_eq!(back.by_model[0].0, "z");
        assert_eq!(back, sum);
    }
}
