//! Docs drift: every subcommand and long flag the real parser accepts must appear in
//! `docs/manual/src/cli.md`, and every config key in `docs/manual/src/config.md`.
//!
//! The command tree is discovered by running the real binary's `--help` recursively, so nothing
//! here is a hand-maintained list. Hidden args and subcommands never show in `--help`, so they are
//! exempt. Clap's built-in `--help` and `--version` and the generated `help` subcommand are
//! skipped: the manual need not document them. Config keys come from `DEFAULT_CONFIG`, which lists
//! every key (the unset `work-branch` as a comment); core exposes no separate key list.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail loudly"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Something the docs must mention verbatim, and where it came from (for the failure message).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    needle: String,
    origin: String,
}

/// What `--help` of one command revealed.
#[derive(Debug, Default, PartialEq, Eq)]
struct HelpScan {
    subcommands: Vec<String>,
    long_flags: Vec<String>,
}

/// Items whose needle does not occur in `doc`, formatted as `needle (origin)`.
fn missing_items(doc: &str, items: &[Item]) -> Vec<String> {
    items
        .iter()
        .filter(|i| !doc.contains(&i.needle))
        .map(|i| format!("{} ({})", i.needle, i.origin))
        .collect()
}

/// Read subcommand names and long flags out of clap's `--help` text.
fn scan_help(help: &str) -> HelpScan {
    let mut scan = HelpScan::default();
    let mut section = "";
    for line in help.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            section = line;
            continue;
        }
        // Entries are indented two spaces; wrapped descriptions are indented deeper.
        let Some(rest) = line
            .strip_prefix("  ")
            .filter(|r| !r.starts_with("  ") || r.trim_start().starts_with('-'))
        else {
            continue;
        };
        if section == "Commands:" {
            if rest.starts_with(' ') {
                continue;
            }
            if let Some(name) = rest.split_whitespace().next()
                && name != "help"
            {
                scan.subcommands.push(name.to_owned());
            }
        } else if section.ends_with("Options:") || section.ends_with("options:") {
            for token in rest.split(|c: char| c.is_whitespace() || c == ',') {
                if token.is_empty() {
                    continue;
                }
                if !token.starts_with('-') {
                    break; // the value name or description starts; ignore prose after it
                }
                if let Some(flag) = token.strip_prefix("--") {
                    let flag = flag.split(['=', '<']).next().unwrap_or(flag);
                    if !flag.is_empty() && flag != "help" && flag != "version" {
                        scan.long_flags.push(format!("--{flag}"));
                    }
                }
            }
        }
    }
    scan
}

fn help_of(path: &[String]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_branch-tally"))
        .args(path)
        .arg("--help")
        .output()
        .expect("run branch-tally --help");
    assert!(out.status.success(), "`{}` --help failed", path.join(" "));
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

/// Walk the real command tree. Every subcommand path and every long flag becomes an [`Item`].
fn cli_items() -> Vec<Item> {
    let mut items = Vec::new();
    let mut stack: Vec<Vec<String>> = vec![Vec::new()];
    while let Some(path) = stack.pop() {
        let scan = scan_help(&help_of(&path));
        let name = if path.is_empty() {
            "branch-tally".to_owned()
        } else {
            path.join(" ")
        };
        for flag in scan.long_flags {
            items.push(Item {
                needle: flag,
                origin: name.clone(),
            });
        }
        for sub in scan.subcommands {
            let mut child = path.clone();
            child.push(sub);
            items.push(Item {
                needle: child.join(" "),
                origin: "subcommand".to_owned(),
            });
            stack.push(child);
        }
    }
    items
}

/// Config keys: every `key = ` line of `DEFAULT_CONFIG`, commented out or not.
fn config_items() -> Vec<Item> {
    branch_tally_core::config::DEFAULT_CONFIG
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("# ").unwrap_or(line);
            let (key, _) = line.split_once(" = ")?;
            let is_key = !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '-');
            is_key.then(|| Item {
                needle: key.to_owned(),
                origin: "config key".to_owned(),
            })
        })
        .collect()
}

fn manual_page(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("manual")
        .join("src")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

fn assert_documented(page: &str, items: &[Item]) {
    assert!(!items.is_empty(), "found nothing to check for {page}");
    let missing = missing_items(&manual_page(page), items);
    assert!(
        missing.is_empty(),
        "docs/manual/src/{page} does not mention: {}. Update that page in the same PR.",
        missing.join(", ")
    );
}

fn item(needle: &str, origin: &str) -> Item {
    Item {
        needle: needle.to_owned(),
        origin: origin.to_owned(),
    }
}

#[test]
fn missing_items_reports_each_absent_needle_with_its_origin() {
    let items = [
        item("--kept", "report"),
        item("--foo", "report"),
        item("prices update", "subcommand"),
    ];
    let missing = missing_items("Use `--kept` here.", &items);
    assert_eq!(missing, ["--foo (report)", "prices update (subcommand)"]);
}

#[test]
fn missing_items_is_empty_when_everything_is_present() {
    assert_eq!(
        missing_items("--a", &[item("--a", "x")]),
        Vec::<String>::new()
    );
}

#[test]
fn scan_help_finds_subcommands_and_long_flags_but_not_builtins() {
    let help = "About\n\nUsage: x <COMMAND>\n\nCommands:\n  report  Mentions --not-a-flag here\n  \
                prices  Manage\n  help    Print this message\n\nOptions:\n  -b, --branch <BRANCH>  \
                Pick\n      --dry-run          Skip\n  -h, --help             Print help\n  \
                -V, --version          Print version\n";
    let scan = scan_help(help);
    assert_eq!(scan.subcommands, ["report", "prices"]);
    assert_eq!(scan.long_flags, ["--branch", "--dry-run"]);
}

#[test]
fn cli_page_mentions_every_subcommand_and_long_flag() {
    assert_documented("cli.md", &cli_items());
}

#[test]
fn config_page_mentions_every_config_key() {
    assert_documented("config.md", &config_items());
}
