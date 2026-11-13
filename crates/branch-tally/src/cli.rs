//! Command-line definition. Parsing only; nothing here runs the pipeline.

use std::path::PathBuf;

use branch_tally_core::config::Format;
use branch_tally_core::model::Mode;
use clap::{Args, Parser, Subcommand, ValueEnum};

/// Version reported by `--version`: the release tag's version, injected at compile time as
/// `BRANCH_TALLY_VERSION` (no leading `v`), or `0.0.0-dev` for any other build.
pub(crate) const VERSION: &str = select_version(option_env!("BRANCH_TALLY_VERSION"));

/// Pick the reported version from the build-time environment value.
const fn select_version(build_env: Option<&'static str>) -> &'static str {
    match build_env {
        Some(v) => v,
        None => "0.0.0-dev",
    }
}

/// Report what a branch or pull request cost in AI-agent tokens.
#[derive(Debug, Parser)]
#[command(name = "branch-tally", version = VERSION, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Report Claude Code token usage for a git branch.
    Report(ReportArgs),

    /// Report project-wide totals and the cost-over-time series from the ledger.
    ///
    /// Reads the ledger only (never the live transcripts) and prints the project total, a
    /// per-branch breakdown and a Mermaid cost-over-time chart. `--export-graph` also writes the
    /// same chart as an SVG file.
    Project(ProjectArgs),

    /// Append this branch's attributed usage to the repository ledger (write only).
    Record(RecordArgs),

    /// Record this session's branch from a Claude Code hook payload on stdin.
    ///
    /// Meant to be run by the capture plugin's hooks (`SessionStart`, `SubagentStop`, `Stop`), not by
    /// hand. Appends an id-only event (session, agent, branch, kind, time) to the main checkout's
    /// .tally/captures.jsonl. No paths or message content are stored. Silent on success; on
    /// failure it prints a short fixed message and exits 1, and the hook command's `; exit 0`
    /// keeps that from blocking the session.
    Capture,

    /// Check the committed `.tally/` files against a base ref.
    #[command(subcommand)]
    Ledger(LedgerCommand),

    /// Manage the model price table.
    #[command(subcommand)]
    Prices(PricesCommand),

    /// Inspect or create the user config file.
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Update branch-tally in place from the latest GitHub release.
    ///
    /// Downloads the release archive for this platform, checks it, and replaces the running
    /// binary. Asks before installing unless `--yes` is given. Development builds and read-only
    /// install locations are refused. Installs made by a package manager should be updated with
    /// that package manager instead.
    ///
    /// What is checked, in order: SHA256SUMS.minisig must be a valid minisign signature over
    /// SHA256SUMS, made by the maintainer's key compiled into this binary, and naming the release
    /// being installed. That signature is the proof that the release was published with the
    /// maintainer's key. Only then is the archive compared with its SHA256SUMS entry, which is an
    /// integrity check against a corrupted download, not proof of origin. The key cannot be
    /// replaced at run time, so a mirror (`BRANCH_TALLY_INSTALL_BASE_URL`) is accepted: it is
    /// trusted only as far as the signature.
    ///
    /// A missing, malformed or mismatched signature installs nothing. Releases from before signing
    /// are refused, including with `--version`. To install one anyway, download it and check it by
    /// hand with `gh attestation verify` (see SECURITY.md).
    #[command(alias = "upgrade")]
    Update(UpdateArgs),
}

#[derive(Debug, Subcommand)]
pub(crate) enum LedgerCommand {
    /// Fail if the head's `.tally/` files are not a well-formed append to the base's.
    ///
    /// Compares `.tally/ledger.jsonl` and `.tally/captures.jsonl` on the head (the working tree,
    /// or the rev given by `--head`) with the same files on `--base`. Every base line must still
    /// be there; every new line must parse, not be recorded for the base branch, carry a valid
    /// non-future `recorded_at`, not repeat a base request, and stay under 10,000,000 tokens per
    /// field. New lines recorded for a branch that is neither the head branch nor the base
    /// branch are noted on stderr, not failed. A difference in `.tally/prices.toml` is a
    /// warning. Violations are listed on stderr (file, line, rule; never line text) and the
    /// exit status is 1.
    Check(LedgerCheckArgs),
}

#[derive(Debug, Args)]
pub(crate) struct LedgerCheckArgs {
    /// The ref to compare against (for example `origin/main`). Required.
    #[arg(long, value_name = "REF")]
    pub(crate) base: String,

    /// The head branch: new lines recorded for any other non-base branch get a note (default: the
    /// current branch; required on a detached checkout).
    #[arg(long, value_name = "NAME")]
    pub(crate) head_branch: Option<String>,

    /// The ref whose `.tally/` files are the head (default: the working tree). Read from git, so
    /// the working tree is not consulted.
    #[arg(long, value_name = "REV")]
    pub(crate) head: Option<String>,

    /// The base branch's name, which new lines must not be recorded for (default: the
    /// repository's default branch; there is no guess if that cannot be found).
    #[arg(long, value_name = "NAME")]
    pub(crate) base_branch: Option<String>,

    /// Repository path (default: cwd).
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub(crate) repo: PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct UpdateArgs {
    /// Only report whether a different version is available; install nothing.
    #[arg(long)]
    pub(crate) check: bool,

    /// Install this release (`v1.2.3` or `1.2.3`) instead of the latest. May downgrade.
    #[arg(long, value_name = "vX.Y.Z")]
    pub(crate) version: Option<String>,

    /// Do not ask for confirmation.
    #[arg(long)]
    pub(crate) yes: bool,

    /// Reinstall even when already on the target version.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Show the config file path, whether it was found, and each effective value and its source.
    Show(ConfigShowArgs),

    /// Write a config file holding the built-in defaults. Never overwrites an existing file.
    Init(ConfigInitArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ConfigInitArgs {
    /// Where to write it (default: the user config path that `config show` prints).
    #[arg(long, value_name = "PATH")]
    pub(crate) path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigShowArgs {
    /// Read this config file instead of the default location (it must exist).
    #[arg(long, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,
}

/// `--mode` choices.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ModeArg {
    Claim,
    Strict,
    Timeline,
}

/// `--format` choices.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FormatArg {
    Md,
    Json,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PricesCommand {
    /// Refresh the per-user price cache from the `LiteLLM` model price list.
    Update,

    /// Show which price table reports would use, with its source, date and path.
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ShowArgs {
    /// Repository path (default: cwd), used to find `.tally/prices.toml`.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub(crate) repo: PathBuf,
}

/// A `--work-branch` pattern: the text as given (the report shows it) and its compiled form.
#[derive(Debug, Clone)]
pub(crate) struct WorkBranchPattern {
    pub(crate) text: String,
    regex: regex_lite::Regex,
}

impl WorkBranchPattern {
    /// Unanchored, as written: `^(feat|refactor)/` works as users expect.
    pub(crate) fn matches(&self, branch: &str) -> bool {
        self.regex.is_match(branch)
    }
}

pub(crate) fn parse_work_branch(text: &str) -> Result<WorkBranchPattern, String> {
    regex_lite::Regex::new(text)
        .map(|regex| WorkBranchPattern {
            text: text.to_owned(),
            regex,
        })
        .map_err(|e| e.to_string())
}

#[derive(Debug, Args)]
// Each flag is an independent switch of the public CLI contract, not a state machine.
#[expect(clippy::struct_excessive_bools, reason = "one bool per CLI flag")]
pub(crate) struct ReportArgs {
    /// Branch to report on (default: current branch).
    pub(crate) branch: Option<String>,

    /// Repository path (default: cwd).
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub(crate) repo: PathBuf,

    /// Attribution mode: claim (default), strict or timeline. `--strict` and `--timeline` are
    /// shorthands; they cannot be combined with `--mode`.
    #[arg(long, value_enum, conflicts_with_all = ["strict", "timeline"])]
    pub(crate) mode: Option<ModeArg>,

    /// Naive gitBranch filter: no session claim, no worktree recovery (same as --mode strict).
    #[arg(long)]
    pub(crate) strict: bool,

    /// Attribute by each session's branch timeline instead of its single claimed branch (same
    /// as --mode timeline).
    #[arg(long, conflicts_with = "strict")]
    pub(crate) timeline: bool,

    /// With timeline mode, a regex naming the work branches (default: every branch except the
    /// repository's default branch and worktree-agent-*). Matched anywhere in the branch name,
    /// as written; anchor it with ^ and $ to match the whole name.
    #[arg(long, value_name = "REGEX", value_parser = parse_work_branch)]
    pub(crate) work_branch: Option<WorkBranchPattern>,

    /// Output format: md (default) or json. `--md` and `--json` are shorthands; they cannot be
    /// combined with `--format`.
    #[arg(long, value_enum, conflicts_with_all = ["md", "json"])]
    pub(crate) format: Option<FormatArg>,

    /// Emit Markdown (the default; same as --format md).
    #[arg(long, conflicts_with = "json")]
    pub(crate) md: bool,

    /// Emit JSON instead of Markdown (same as --format json).
    #[arg(long)]
    pub(crate) json: bool,

    /// Splice the table into PR N (default: this branch's open PR).
    #[arg(long, num_args = 0..=1, default_missing_value = "", value_name = "N")]
    pub(crate) pr: Option<String>,

    /// With --pr, print the body without editing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Omit the estimated-cost column.
    #[arg(long, conflicts_with = "cost")]
    pub(crate) no_cost: bool,

    /// Show the estimated-cost column (the default); undoes `cost = false` in the config file.
    #[arg(long)]
    pub(crate) cost: bool,

    /// Hide identifiers in the output: session ids become s1, s2, ..., agent and skill names
    /// become generic labels, and timestamps are reduced to dates. Costs stay visible.
    #[arg(long, conflicts_with = "no_redact")]
    pub(crate) redact: bool,

    /// Show identifiers (the default); undoes `redact = true` in the config file.
    #[arg(long)]
    pub(crate) no_redact: bool,

    /// With redaction, keep real agent and skill names; ids and times are still hidden. Has no
    /// effect when redaction is off.
    #[arg(long, conflicts_with = "redact_names")]
    pub(crate) keep_names: bool,

    /// With redaction, replace agent and skill names with generic labels (the default); undoes
    /// `redact-names = false` in the config file.
    #[arg(long)]
    pub(crate) redact_names: bool,

    /// Read this config file instead of the default location (it must exist).
    #[arg(long, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    /// Report from the ledger instead of the live transcripts (works with no transcripts).
    #[arg(long)]
    pub(crate) from_ledger: bool,

    /// With --from-ledger, also union in this machine's live transcripts (the ledger wins on
    /// overlap, so nothing is counted twice).
    #[arg(long, requires = "from_ledger")]
    pub(crate) with_live: bool,

    /// The ledger file (default: .tally/ledger.jsonl at the root of the current checkout).
    #[arg(long, value_name = "PATH")]
    pub(crate) ledger: Option<PathBuf>,

    /// With --from-ledger, proceed even if the ledger has conflicting lines for one request (the latest line wins).
    /// Without it such a ledger stops the run with exit 3, because a conflict can rewrite cost.
    #[arg(long, requires = "from_ledger")]
    pub(crate) allow_ledger_conflicts: bool,
}

/// `project`: the ledger-wide report.
#[derive(Debug, Args)]
pub(crate) struct ProjectArgs {
    /// Repository path (default: cwd), used to find the default ledger and the price table.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub(crate) repo: PathBuf,

    /// The ledger file (default: .tally/ledger.jsonl at the root of the current checkout).
    #[arg(long, value_name = "PATH")]
    pub(crate) ledger: Option<PathBuf>,

    /// Proceed even if the ledger has conflicting lines for one request (the latest line wins).
    /// Without it such a ledger stops the run with exit 3, because a conflict can rewrite cost.
    #[arg(long)]
    pub(crate) allow_ledger_conflicts: bool,

    /// Output format: md (default) or json. `--md` and `--json` are shorthands; they cannot be
    /// combined with `--format`.
    #[arg(long, value_enum, conflicts_with_all = ["md", "json"])]
    pub(crate) format: Option<FormatArg>,

    /// Emit Markdown (the default; same as --format md).
    #[arg(long, conflicts_with = "json")]
    pub(crate) md: bool,

    /// Emit JSON instead of Markdown (same as --format json).
    #[arg(long)]
    pub(crate) json: bool,

    /// Read this config file instead of the default location (it must exist).
    #[arg(long, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    /// Also write the cost-over-time chart as an SVG file to PATH (the report still prints). The
    /// file is written whole or not at all; an existing file is replaced.
    #[arg(long, value_name = "PATH")]
    pub(crate) export_graph: Option<PathBuf>,
}

impl ProjectArgs {
    /// The format a flag chose (`--format`, `--md` or `--json`), or `None` when none did.
    pub(crate) fn format_flag(&self) -> Option<Format> {
        format_from(self.format, self.md, self.json)
    }
}

fn format_from(format: Option<FormatArg>, md: bool, json: bool) -> Option<Format> {
    match format {
        Some(FormatArg::Md) => Some(Format::Md),
        Some(FormatArg::Json) => Some(Format::Json),
        None if json => Some(Format::Json),
        None if md => Some(Format::Md),
        None => None,
    }
}

/// `record`: the same selection flags as `report`, then append to the ledger.
#[derive(Debug, Args)]
pub(crate) struct RecordArgs {
    /// Branch to record (default: current branch).
    pub(crate) branch: Option<String>,

    /// Repository path (default: cwd).
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub(crate) repo: PathBuf,

    /// Attribution mode: claim (default), strict or timeline.
    #[arg(long, value_enum, conflicts_with_all = ["strict", "timeline"])]
    pub(crate) mode: Option<ModeArg>,

    /// Same as --mode strict.
    #[arg(long)]
    pub(crate) strict: bool,

    /// Same as --mode timeline.
    #[arg(long, conflicts_with = "strict")]
    pub(crate) timeline: bool,

    /// With timeline mode, a regex naming the work branches (see `report --help`).
    #[arg(long, value_name = "REGEX", value_parser = parse_work_branch)]
    pub(crate) work_branch: Option<WorkBranchPattern>,

    /// Read this config file instead of the default location (it must exist).
    #[arg(long, value_name = "PATH")]
    pub(crate) config: Option<PathBuf>,

    /// The ledger file (default: .tally/ledger.jsonl at the root of the current checkout).
    #[arg(long, value_name = "PATH")]
    pub(crate) ledger: Option<PathBuf>,
}

impl RecordArgs {
    /// The mode a flag chose, or `None`.
    pub(crate) fn mode_flag(&self) -> Option<Mode> {
        mode_from(self.mode, self.strict, self.timeline)
    }
}

fn mode_from(mode: Option<ModeArg>, strict: bool, timeline: bool) -> Option<Mode> {
    match mode {
        Some(ModeArg::Claim) => Some(Mode::Claim),
        Some(ModeArg::Strict) => Some(Mode::Strict),
        Some(ModeArg::Timeline) => Some(Mode::Timeline),
        None if strict => Some(Mode::Strict),
        None if timeline => Some(Mode::Timeline),
        None => None,
    }
}

impl ReportArgs {
    /// The mode a flag chose (`--mode`, `--strict` or `--timeline`), or `None` when none did.
    pub(crate) fn mode_flag(&self) -> Option<Mode> {
        mode_from(self.mode, self.strict, self.timeline)
    }

    /// The format a flag chose (`--format`, `--md` or `--json`), or `None` when none did.
    pub(crate) fn format_flag(&self) -> Option<Format> {
        format_from(self.format, self.md, self.json)
    }

    /// `--cost` or `--no-cost`, or `None` when neither was given.
    pub(crate) fn cost_flag(&self) -> Option<bool> {
        match (self.cost, self.no_cost) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }

    /// `--redact` or `--no-redact`, or `None` when neither was given.
    pub(crate) fn redact_flag(&self) -> Option<bool> {
        match (self.redact, self.no_redact) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }

    /// `--redact-names` (true) or `--keep-names` (false), or `None` when neither was given.
    pub(crate) fn redact_names_flag(&self) -> Option<bool> {
        match (self.redact_names, self.keep_names) {
            (true, _) => Some(true),
            (_, true) => Some(false),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::select_version;

    #[test]
    fn build_env_value_wins() {
        assert_eq!(select_version(Some("1.2.3")), "1.2.3");
    }

    #[test]
    fn unset_build_env_is_dev() {
        assert_eq!(select_version(None), "0.0.0-dev");
    }
}
