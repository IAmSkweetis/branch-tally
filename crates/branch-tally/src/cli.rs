//! Command-line definition. Parsing only; nothing here runs the pipeline.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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

fn parse_work_branch(text: &str) -> Result<WorkBranchPattern, String> {
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

    /// Naive gitBranch filter: no session claim, no worktree recovery.
    #[arg(long)]
    pub(crate) strict: bool,

    /// Attribute by each session's branch timeline instead of its single claimed branch.
    #[arg(long, conflicts_with = "strict")]
    pub(crate) timeline: bool,

    /// With --timeline, a regex naming the work branches (default: every branch except the
    /// repository's default branch and worktree-agent-*). Matched anywhere in the branch name,
    /// as written; anchor it with ^ and $ to match the whole name.
    #[arg(long, requires = "timeline", value_name = "REGEX", value_parser = parse_work_branch)]
    pub(crate) work_branch: Option<WorkBranchPattern>,

    /// Emit Markdown (the default).
    #[arg(long, conflicts_with = "json")]
    pub(crate) md: bool,

    /// Emit JSON instead of Markdown.
    #[arg(long)]
    pub(crate) json: bool,

    /// Splice the table into PR N (default: this branch's open PR).
    #[arg(long, num_args = 0..=1, default_missing_value = "", value_name = "N")]
    pub(crate) pr: Option<String>,

    /// With --pr, print the body without editing.
    #[arg(long)]
    pub(crate) dry_run: bool,

    /// Omit the estimated-cost column.
    #[arg(long)]
    pub(crate) no_cost: bool,
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
