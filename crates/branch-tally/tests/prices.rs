//! End-to-end tests for the price table: lookup order, the `prices` subcommand and the unpriced
//! warning. No test reaches the real network: `prices update` talks to a loopback server through
//! the test-only `BRANCH_TALLY_PRICES_URL` variable.

#![expect(
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns fail loudly on setup errors"
)]

#[expect(
    dead_code,
    reason = "helpers shared with the other end-to-end test crates"
)]
mod common;

use common::{World, stderr, stdout};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

/// A table that prices every `priced-models` model at one dollar per million tokens, dated in
/// the future so no other table can be mistaken for it.
const OVERRIDE: &str = r#"
as_of = "2030-01-02"

[models."claude-opus-5"]
input = 1.0
output = 1.0

[models."claude-sonnet-4-6"]
input = 1.0
output = 1.0

[models."claude-fable-5-1"]
input = 1.0
output = 1.0

[models."claude-mystery-9"]
input = 1.0
output = 1.0
"#;

const CACHE: &str =
    "as_of = \"2029-05-06\"\n\n[models.\"claude-mystery-9\"]\ninput = 2.0\noutput = 2.0\n";

const FEED: &str = r#"{
  "sample_spec": {"litellm_provider": "x"},
  "claude-test-1": {"litellm_provider": "anthropic", "mode": "chat",
    "input_cost_per_token": 3e-6, "output_cost_per_token": 15e-6,
    "cache_read_input_token_cost": 3e-7, "cache_creation_input_token_cost": 3.75e-6},
  "claude-broken-1": {"litellm_provider": "anthropic", "mode": "chat"}
}"#;

impl World {
    /// Where `prices update` writes the cache file in this world.
    fn cache_file(&self) -> std::path::PathBuf {
        self.cache_root().join("branch-tally").join("prices.toml")
    }

    /// Writes `.tally/prices.toml` in the repository.
    fn write_override(&self, text: &str) {
        let dir = self.repo.join(".tally");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("prices.toml"), text).unwrap();
    }

    /// Runs `args` with the price feed pointed at `url` (the test-only `BRANCH_TALLY_PRICES_URL`).
    fn run_with_feed(&self, url: &str, args: &[&str]) -> std::process::Output {
        self.command(&self.repo, args)
            .env("BRANCH_TALLY_PRICES_URL", url)
            .output()
            .unwrap()
    }
}

fn json(world: &World, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["report", "main", "--json"];
    args.extend_from_slice(extra);
    let out = world.run(&args);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let mut payload: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    world.normalize(&mut payload);
    payload
}

/// Serves `status` and `body` to one request on a loopback port; returns the URL and the thread.
fn serve_once(status: &str, body: &str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/prices.json", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let handle = std::thread::spawn(move || {
        // Poll rather than block, so a client that never connects fails its own assertions
        // instead of hanging the suite.
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => return,
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&buf[..n]);
        }
        stream.write_all(response.as_bytes()).unwrap();
    });
    (url, handle)
}

#[test]
fn override_changes_the_cost_and_the_footnote_source() {
    let world = World::scenario("priced-models");
    let bundled = json(&world, &[]);
    assert_eq!(bundled["priceTable"]["source"], "bundled");

    world.write_override(OVERRIDE);
    let got = json(&world, &[]);
    assert_eq!(got["priceTable"]["source"], "override");
    assert_eq!(got["priceTable"]["asOf"], "2030-01-02");
    assert_ne!(got["totalCostUSD"], bundled["totalCostUSD"]);
    assert_eq!(got["totalCostIsFloor"], false);

    let md = world.run(&["report", "main"]);
    let text = stdout(&md);
    assert!(
        text.contains("prices as of 2030-01-02 (.tally/prices.toml)"),
        "{text}"
    );
}

#[test]
fn the_override_is_found_from_a_linked_worktree_via_the_main_checkout() {
    let world = World::scenario("subagent");
    world.write_override(&OVERRIDE.replace("2030-01-02", "2031-03-04"));
    let wt = world
        .repo
        .join(".claude")
        .join("worktrees")
        .join("agent-a1");
    let out = world.run_in(&wt, &["report", "feat/x", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let got: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(got["priceTable"]["asOf"], "2031-03-04");
}

#[test]
fn a_malformed_override_exits_1_naming_the_path_and_not_the_content() {
    let world = World::scenario("priced-models");
    world.write_override("as_of = \"SECRET-NOT-A-DATE\"\n");
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    let err = stderr(&out);
    assert!(err.contains("prices.toml"), "{err}");
    assert!(err.contains(".tally"), "{err}");
    assert!(!err.contains("SECRET"), "{err}");
    assert_eq!(err.lines().count(), 1, "{err}");
}

#[test]
fn a_cache_file_is_used_when_there_is_no_override_and_the_override_beats_it() {
    let world = World::scenario("priced-models");
    fs::create_dir_all(world.cache_file().parent().unwrap()).unwrap();
    fs::write(world.cache_file(), CACHE).unwrap();
    let cached = json(&world, &[]);
    assert_eq!(cached["priceTable"]["source"], "cache");
    assert_eq!(cached["priceTable"]["asOf"], "2029-05-06");

    world.write_override(OVERRIDE);
    let over = json(&world, &[]);
    assert_eq!(over["priceTable"]["source"], "override");
}

#[test]
fn a_malformed_cache_exits_1_naming_the_path() {
    let world = World::scenario("priced-models");
    fs::create_dir_all(world.cache_file().parent().unwrap()).unwrap();
    fs::write(world.cache_file(), "not toml [[[").unwrap();
    let out = world.run(&["report", "main"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("prices.toml"));
}

#[test]
fn unpriced_models_are_named_in_a_stderr_warning_with_the_table_date() {
    let world = World::scenario("priced-models");
    let out = world.run(&["report", "main", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let err = stderr(&out);
    let date =
        serde_json::from_str::<serde_json::Value>(&stdout(&out)).unwrap()["priceTable"]["asOf"]
            .as_str()
            .unwrap()
            .to_owned();
    assert!(
        err.contains(&format!(
            "warning: no price for claude-mystery-9 in the price table (as of {date}); the total is a floor\n"
        )),
        "{err}"
    );
    // A fully priced report is silent.
    world.write_override(OVERRIDE);
    assert_eq!(stderr(&world.run(&["report", "main"])), "");
}

#[test]
fn prices_update_writes_the_dated_cache_from_the_feed() {
    let world = World::scenario("priced-models");
    let (url, server) = serve_once("200 OK", FEED);
    let out = world.run_with_feed(&url, &["prices", "update"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = fs::read_to_string(world.cache_file()).unwrap();
    assert!(!text.contains('\r'));
    assert!(text.contains("[models.\"claude-test-1\"]"), "{text}");
    assert!(!text.contains("claude-broken-1"));
    let said = stdout(&out);
    assert!(
        said.contains(&world.cache_file().display().to_string()),
        "{said}"
    );
    assert!(said.contains("models: 1\n"), "{said}");
    assert!(said.contains("skipped"), "{said}");
    assert!(said.contains("claude-broken-1"), "{said}");

    // The cache now wins over the bundled table in reports.
    let got = json(&world, &[]);
    assert_eq!(got["priceTable"]["source"], "cache");
    let as_of = got["priceTable"]["asOf"].as_str().unwrap();
    assert!(
        as_of.len() == 10 && as_of.chars().filter(|c| *c == '-').count() == 2,
        "{as_of}"
    );
}

#[test]
fn a_failing_feed_exits_1_and_leaves_an_existing_cache_untouched() {
    let world = World::scenario("priced-models");
    fs::create_dir_all(world.cache_file().parent().unwrap()).unwrap();
    fs::write(world.cache_file(), CACHE).unwrap();
    for (status, body) in [
        ("500 Internal Server Error", "boom"),
        ("200 OK", "not json"),
    ] {
        let (url, server) = serve_once(status, body);
        let out = world.run_with_feed(&url, &["prices", "update"]);
        server.join().unwrap();
        assert_eq!(out.status.code(), Some(1), "{status}");
        assert_eq!(stdout(&out), "");
        assert!(!stderr(&out).contains("boom"));
        assert_eq!(fs::read_to_string(world.cache_file()).unwrap(), CACHE);
    }
}

#[test]
fn an_unreachable_feed_exits_1_without_creating_a_cache() {
    let world = World::empty();
    // Bind then drop, so the port is closed.
    let url = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/prices.json", l.local_addr().unwrap())
    };
    let out = world.run_with_feed(&url, &["prices", "update"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("price feed unavailable"),
        "{}",
        stderr(&out)
    );
    assert!(!world.cache_file().exists());
}

#[test]
fn prices_show_reports_the_table_in_use() {
    let world = World::empty();
    let out = world.run(&["prices", "show"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("source: bundled"), "{text}");
    assert!(text.contains("as of: "), "{text}");

    world.write_override(OVERRIDE);
    let text = stdout(&world.run(&["prices", "show"]));
    assert!(text.contains("source: override"), "{text}");
    assert!(text.contains("as of: 2030-01-02"), "{text}");
    assert!(text.contains("prices.toml"), "{text}");
    assert!(text.contains("models: 4"), "{text}");
}

#[test]
fn prices_needs_a_subcommand_and_help_lists_update_and_show() {
    let world = World::empty();
    let out = world.run(&["prices"]);
    assert_eq!(out.status.code(), Some(2));
    let help = world.run(&["prices", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help);
    assert!(text.contains("update") && text.contains("show"), "{text}");
}
