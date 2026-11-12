//! The `branch-tally` command-line binary. A thin wrapper over `branch-tally-core`.
//!
//! The flags, exit codes and output shapes are a public contract (scripts, the plugin skill and
//! the Action read them). Exit 0 on success, 1 for a failure with a message on stderr, 2 for a
//! command-line error (clap).

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

mod capture;
mod cli;
mod config;
mod prices;
mod project;
mod pyrepr;
mod report;
mod update;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let result = match args.command {
        cli::Command::Report(args) => report::run(&args),
        cli::Command::Project(args) => project::run(&args),
        cli::Command::Record(args) => report::record(&args),
        cli::Command::Capture => capture::run(),
        cli::Command::Prices(command) => prices::run(&command).map(|text| {
            // Verbatim UTF-8 with LF, whatever the console codepage.
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }),
        cli::Command::Update(args) => update::run(&args).map(|message| {
            let _ = writeln!(std::io::stdout().lock(), "{message}");
        }),
        cli::Command::Config(command) => match command {
            cli::ConfigCommand::Show(args) => config::show(&args),
            cli::ConfigCommand::Init(args) => config::init(&args),
        }
        .map(|text| {
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
