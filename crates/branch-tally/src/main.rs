//! The `branch-tally` command-line binary. A thin wrapper over `branch-tally-core`.
//!
//! The flags, exit codes and output shapes are a public contract (scripts, the plugin skill and
//! the Action read them). Exit 0 on success, 1 for a failure with a message on stderr, 2 for a
//! command-line error (clap).

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

mod cli;
mod pyrepr;
mod report;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    match args.command {
        cli::Command::Report(args) => match report::run(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                // Python's `sys.exit(msg)`: the message and a newline on stderr, status 1.
                let _ = writeln!(std::io::stderr(), "{message}");
                ExitCode::from(1)
            }
        },
    }
}
