//! Command-line definition. Parsing only; nothing here runs the pipeline.

use std::path::PathBuf;

use branch_tally_core::config::Format;
use branch_tally_core::model::Mode;
use clap::{Args, Parser, Subcommand, ValueEnum};

/// Version reported by `--version`: the release tag's version, injected at compile time as
/// `BRANCH_TALLY_VERSION` (no leading `v`), or `0.0.0-dev` for any other build.
const VERSION: &str = select_version(option_env!("BRANCH_TALLY_VERSION"));

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

    /// Manage the model price table.
    #[command(subcommand)]
    Prices(PricesCommand),

    /// Inspect or create the user config file.
    #[command(subcommand)]
    Config(ConfigCommand),
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
}

impl ReportArgs {
    /// The mode a flag chose (`--mode`, `--strict` or `--timeline`), or `None` when none did.
    pub(crate) fn mode_flag(&self) -> Option<Mode> {
        match self.mode {
            Some(ModeArg::Claim) => Some(Mode::Claim),
            Some(ModeArg::Strict) => Some(Mode::Strict),
            Some(ModeArg::Timeline) => Some(Mode::Timeline),
            None if self.strict => Some(Mode::Strict),
            None if self.timeline => Some(Mode::Timeline),
            None => None,
        }
    }

    /// The format a flag chose (`--format`, `--md` or `--json`), or `None` when none did.
    pub(crate) fn format_flag(&self) -> Option<Format> {
        match self.format {
            Some(FormatArg::Md) => Some(Format::Md),
            Some(FormatArg::Json) => Some(Format::Json),
            None if self.json => Some(Format::Json),
            None if self.md => Some(Format::Md),
            None => None,
        }
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
