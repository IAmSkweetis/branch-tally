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

use super::{SessionId, SubagentSummary};

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
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read + self.output
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
        self.calls += 1;
        self.tokens += tokens;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn footnote_without_heuristics_still_loads() {
        let json = r#"{"subagent_calls":1,"off_branch_tokens":2,"worktree_sessions":3}"#;
        let f: Footnote = serde_json::from_str(json).unwrap();
        assert_eq!(f.heuristics, HeuristicBreakdown::default());
        assert_eq!(f.worktree_sessions, 3);
        let back: Footnote = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
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
