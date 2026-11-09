//! Command-line definition. Parsing only; nothing here runs the pipeline.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Report what a branch or pull request cost in AI-agent tokens.
#[derive(Debug, Parser)]
#[command(name = "branch-tally", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Report Claude Code token usage for a git branch.
    Report(ReportArgs),
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
