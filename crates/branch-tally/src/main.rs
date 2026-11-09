//! The `branch-tally` command-line binary. A thin wrapper over `branch-tally-core`.
//!
//! The flags, exit codes and output shapes are a public contract (scripts, the plugin skill and
//! the Action read them). Exit 0 on success, 1 for a failure with a message on stderr, 2 for a
//! command-line error (clap).

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

mod cli;
mod prices;
mod pyrepr;
mod report;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let result = match args.command {
        cli::Command::Report(args) => report::run(&args),
        cli::Command::Prices(command) => prices::run(&command).map(|text| {
            // Verbatim UTF-8 with LF, whatever the console codepage.
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Python's `sys.exit(msg)`: the message and a newline on stderr, status 1.
            let _ = writeln!(std::io::stderr(), "{message}");
            ExitCode::from(1)
        }
    }
}
