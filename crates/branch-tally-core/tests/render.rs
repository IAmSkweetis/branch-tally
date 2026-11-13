//! Byte-exact checks of `render` against `reference/token_report.py`'s `render`, `wrap`, `splice`
//! and `as_json`, from hand-built precomputed inputs. Expected strings were derived by hand from
//! the reference's format strings, not copied from this crate's output.

use std::collections::BTreeSet;

use branch_tally_core::model::{
    Bucket, Costs, Footnote, HeuristicBreakdown, HeuristicCount, Mode, PriceTableInfo,
    PriceTableSource, SessionId, SessionRow, Summary, Unstamped,
};
use branch_tally_core::render::{END, HEADING, START, json, markdown, splice, wrap};

fn bucket(input: u64, w5: u64, w1h: u64, read: u64, output: u64, think: u64, calls: u64) -> Bucket {
    Bucket {
        input,
        cache_write_5m: w5,
        cache_write_1h: w1h,
        cache_read: read,
        output,
        thinking: think,
        calls,
    }
}

fn session(
    first: &str,
    last: &str,
    origin: &str,
    tokens: u64,
    calls: u64,
    agents: &[&str],
) -> SessionRow {
    let mut row = SessionRow::new(first, origin);
    last.clone_into(&mut row.last);
    row.tokens = tokens;
    row.calls = calls;
    row.agents = agents
        .iter()
        .map(|a| (*a).to_owned())
        .collect::<BTreeSet<_>>();
    row
}

/// Models in first-seen order `claude-a` (output 100) then `claude-b` (output 300), so the
/// reference's `-output` sort must flip them. Sessions are first-seen `abcdefghijkl` then `s2`,
/// which sorts the other way by `first`. Attributions tie on tokens, to pin stability.
fn sample() -> (Summary, Costs) {
    let summary = Summary {
        branch: "feat/x".to_owned(),
        mode: Mode::Claim,
        total: bucket(1005, 200, 50, 3000, 400, 10, 3),
        footnote: Footnote {
            subagent_calls: 1234,
            off_branch_tokens: 2_500_000,
            worktree_sessions: 1,
            ..Footnote::default()
        },
        by_model: vec![
            (
                "claude-a".to_owned(),
                bucket(1000, 200, 50, 3000, 100, 10, 2),
            ),
            ("claude-b".to_owned(), bucket(5, 0, 0, 0, 300, 0, 1)),
        ],
        by_attribution: vec![
            ("main".to_owned(), bucket(10, 0, 0, 0, 0, 0, 2)),
            ("agent:x".to_owned(), bucket(0, 0, 0, 0, 90, 0, 1)),
            ("skill".to_owned(), bucket(10, 0, 0, 0, 0, 0, 1)),
        ],
        sessions: vec![
            (
                SessionId::new("abcdefghijkl"),
                session(
                    "2026-01-02T03:04:05Z",
                    "2026-01-02T05:06:07Z",
                    "main",
                    1_234_567,
                    2,
                    &[],
                ),
            ),
            (
                SessionId::new("s2"),
                session(
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T10:30:00Z",
                    "d--x--claude-worktrees-y",
                    50_000,
                    1,
                    &["x", "a"],
                ),
            ),
        ],
        subagents: branch_tally_core::model::SubagentSummary::default(),
        work_branch: None,
        unstamped: Unstamped::default(),
        redacted: false,
    };
    // 0.125 and 2.675 are ties for two decimals: 0.125 is exact (half-even gives 0.12), 2.675 is
    // 2.67499999... in binary (gives 2.67).
    let costs = Costs {
        by_model: vec![
            ("claude-a".to_owned(), Some(0.125)),
            ("claude-b".to_owned(), None),
        ],
        total: 2.675,
        unpriced_models: 1,
        ..Costs::default()
    };
    (summary, costs)
}

const TABLES_HEAD: &str =
    "| Model | Calls | Input | Cache write | Cache read | Output | (thinking) |";

fn tail(note_end: &str) -> String {
    format!(
        "\n\n| Session | Where | Agents | Span (UTC) | Calls | Tokens |\n\
         |---|---|---|---|--:|--:|\n\
         | `s2` | worktree | `a`, `x` | 01-01 10:00 \u{2192} 01-01 10:30 | 1 | 0.1M |\n\
         | `abcdefgh` | main checkout | \u{2014} | 01-02 03:04 \u{2192} 01-02 05:06 | 2 | 1.2M |\n\
         \n\
         <sub>Branch `feat/x` \u{b7} 2 session(s), 1 in a worktree \u{b7} 1,234 of 3 calls from \
         subagents{note_end}\n\
         \n\
         <details><summary>Breakdown by skill / agent</summary>\n\
         \n\
         | Attribution | Calls | Tokens |\n\
         |---|--:|--:|\n\
         | `agent:x` | 1 | 90 |\n\
         | `main` | 2 | 10 |\n\
         | `skill` | 1 | 10 |\n\
         \n\
         </details>"
    )
}

#[test]
fn markdown_with_cost_matches_the_reference_layout() {
    let (s, c) = sample();
    let expected = format!(
        "{TABLES_HEAD} Est. cost |\n\
         |---|--:|--:|--:|--:|--:|--:|--:|\n\
         | `claude-b` | 1 | 5 | 0 | 0 | 300 | 0 | \u{2014} |\n\
         | `claude-a` | 2 | 1,000 | 250 | 3,000 | 100 | 10 | $0.12 |\n\
         | **Total** | **3** | **1,005** | **250** | **3,000** | **400** | **10** | **at least $2.67** |{}",
        tail(
            " \u{b7} price table date unknown \u{b7} 1 model(s) unpriced, total is a floor \u{b7} \
             est. cost at list API rates, not amounts billed</sub>"
        )
    );
    assert_eq!(markdown(&s, &c, true), expected);
}

#[test]
fn markdown_without_cost_drops_the_column_and_the_estimate_note() {
    let (s, c) = sample();
    let expected = format!(
        "{TABLES_HEAD}\n\
         |---|--:|--:|--:|--:|--:|--:|\n\
         | `claude-b` | 1 | 5 | 0 | 0 | 300 | 0 |\n\
         | `claude-a` | 2 | 1,000 | 250 | 3,000 | 100 | 10 |\n\
         | **Total** | **3** | **1,005** | **250** | **3,000** | **400** | **10** |{}",
        tail("</sub>")
    );
    assert_eq!(markdown(&s, &c, false), expected);
}

#[test]
fn markdown_tolerates_empty_short_and_non_ascii_strings() {
    let mut s = Summary::default();
    s.sessions
        .push((SessionId::new(""), session("", "x", "main", 0, 0, &[])));
    s.sessions.push((
        SessionId::new("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}"),
        session("\u{65e5}\u{672c}", "2026-01-02", "main", 0, 0, &[]),
    ));
    let out = markdown(&s, &Costs::default(), false);
    // ts[5:16] of "" and "x" is "" in Python; of "2026-01-02" it is "01-02".
    assert!(
        out.contains("| `` | main checkout | \u{2014} |  \u{2192}  | 0 | 0.0M |"),
        "{out}"
    );
    assert!(
        out.contains("| `\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}` | main checkout | \u{2014} |  \u{2192} 01-02 |"),
        "{out}"
    );
}

#[test]
fn markdown_of_an_empty_summary_still_has_every_section() {
    let out = markdown(&Summary::default(), &Costs::default(), true);
    assert!(
        out.starts_with(&format!("{TABLES_HEAD} Est. cost |\n|---|")),
        "{out}"
    );
    assert!(
        out.contains("| **Total** | **0** | **0** | **0** | **0** | **0** | **0** | **$0.00** |")
    );
    assert!(
        out.ends_with("| Attribution | Calls | Tokens |\n|---|--:|--:|\n\n</details>"),
        "{out}"
    );
    assert!(!out.ends_with('\n'));
}

fn priced(unpriced: u64, as_of: &str, source: PriceTableSource) -> Costs {
    Costs {
        by_model: vec![("m".to_owned(), Some(1.0))],
        total: 1.0,
        unpriced_models: unpriced,
        price_table: PriceTableInfo {
            as_of: as_of.to_owned(),
            source,
        },
        ..Costs::default()
    }
}

#[test]
fn a_fully_priced_total_is_not_labelled_a_floor() {
    let out = markdown(
        &Summary::default(),
        &priced(0, "2026-11-09", PriceTableSource::Bundled),
        true,
    );
    assert!(out.contains("| **$1.00** |"), "{out}");
    assert!(!out.contains("at least"), "{out}");
    assert!(!out.contains("unpriced"), "{out}");
}

#[test]
fn the_report_states_the_price_table_date_and_source() {
    let cases = [
        (PriceTableSource::Bundled, "bundled table"),
        (PriceTableSource::Cache, "cached from LiteLLM"),
        (PriceTableSource::Override, ".tally/prices.toml"),
    ];
    for (source, label) in cases {
        let out = markdown(&Summary::default(), &priced(0, "2026-11-09", source), true);
        assert!(
            out.contains(&format!(
                " \u{b7} prices as of 2026-11-09 ({label}) \u{b7} est. cost"
            )),
            "{out}"
        );
    }
}

#[test]
fn no_cost_hides_the_price_table_and_floor_notes() {
    let out = markdown(
        &Summary::default(),
        &priced(2, "2026-11-09", PriceTableSource::Cache),
        false,
    );
    assert!(!out.contains("prices as of"), "{out}");
    assert!(!out.contains("at least"), "{out}");
    assert!(!out.contains("unpriced"), "{out}");
}

#[test]
fn json_states_the_price_table_and_names_unpriced_models() {
    let mut s = Summary::default();
    s.by_model.push(("m".to_owned(), Bucket::default()));
    let c = Costs {
        by_model: vec![("m".to_owned(), Some(1.0)), ("zz".to_owned(), None)],
        unpriced_models: 1,
        ..priced(1, "2026-11-09", PriceTableSource::Override)
    };
    let v: serde_json::Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["totalCostIsFloor"], true);
    assert_eq!(v["unpricedModels"], serde_json::json!(["zz"]));
    assert_eq!(v["priceTable"]["asOf"], "2026-11-09");
    assert_eq!(v["priceTable"]["source"], "override");
}

#[test]
fn wrap_puts_markers_on_their_own_lines() {
    assert_eq!(wrap("body"), format!("{START}\nbody\n{END}"));
}

#[test]
fn splice_replaces_the_marked_region_in_place() {
    let body = format!("intro\n{START}\nold\n{END}\noutro");
    assert_eq!(
        splice(&body, "new"),
        format!("intro\n{START}\nnew\n{END}\noutro")
    );
}

#[test]
fn splice_appends_a_heading_when_there_are_no_markers() {
    assert_eq!(
        splice("text  \n\n", "new"),
        format!("text\n\n{HEADING}\n\n{START}\nnew\n{END}\n")
    );
    assert_eq!(
        splice("", "r"),
        format!("\n\n{HEADING}\n\n{START}\nr\n{END}\n")
    );
}

#[test]
fn splice_normalises_crlf_and_ignores_misordered_markers() {
    let body = format!("a\r\n{START}\r\nold\r\n{END}\r\nz\r\n");
    assert_eq!(splice(&body, "n"), format!("a\n{START}\nn\n{END}\nz\n"));
    let reversed = format!("{END}\nmid\n{START}");
    assert_eq!(
        splice(&reversed, "n"),
        format!("{END}\nmid\n{START}\n\n{HEADING}\n\n{START}\nn\n{END}\n")
    );
}

#[test]
fn splice_is_idempotent() {
    let once = splice("pr body", "report");
    assert_eq!(splice(&once, "report"), once);
}

#[test]
fn json_matches_python_dumps_with_indent_2() {
    let (s, c) = sample();
    let expected = r#"{
  "branch": "feat/x",
  "mode": "claim",
  "totalCalls": 3,
  "totalCostUSD": 2.675,
  "totalCostIsFloor": true,
  "unpricedModels": [
    "claude-b"
  ],
  "priceTable": {
    "asOf": null,
    "source": "bundled"
  },
  "sessions": [
    {
      "id": "s2",
      "origin": "d--x--claude-worktrees-y",
      "first": "2026-01-01T10:00:00Z",
      "last": "2026-01-01T10:30:00Z",
      "calls": 1,
      "tokens": 50000,
      "agents": [
        "a",
        "x"
      ]
    },
    {
      "id": "abcdefghijkl",
      "origin": "main",
      "first": "2026-01-02T03:04:05Z",
      "last": "2026-01-02T05:06:07Z",
      "calls": 2,
      "tokens": 1234567,
      "agents": []
    }
  ],
  "models": {
    "claude-b": {
      "calls": 1,
      "input": 5,
      "cacheWrite5m": 0,
      "cacheWrite1h": 0,
      "cacheRead": 0,
      "output": 300,
      "thinking": 0,
      "costUSD": null
    },
    "claude-a": {
      "calls": 2,
      "input": 1000,
      "cacheWrite5m": 200,
      "cacheWrite1h": 50,
      "cacheRead": 3000,
      "output": 100,
      "thinking": 10,
      "costUSD": 0.125
    }
  }
}"#;
    assert_eq!(json(&s, &c), expected);
}

#[test]
fn json_of_an_empty_summary_uses_empty_containers_and_float_zero() {
    assert_eq!(
        json(&Summary::default(), &Costs::default()),
        "{\n  \"branch\": \"\",\n  \"mode\": \"claim\",\n  \"totalCalls\": 0,\n  \
         \"totalCostUSD\": 0.0,\n  \"totalCostIsFloor\": false,\n  \"unpricedModels\": [],\n  \
         \"priceTable\": {\n    \"asOf\": null,\n    \"source\": \"bundled\"\n  },\n  \
         \"sessions\": [],\n  \"models\": {}\n}"
    );
}

#[test]
fn json_strict_mode_and_ascii_escaping() {
    let s = Summary {
        branch: "q\"\\/\n\t\u{1}\u{7f}\u{e9}\u{1f600}".to_owned(),
        mode: Mode::Strict,
        ..Summary::default()
    };
    let out = json(&s, &Costs::default());
    assert!(out.contains("\"mode\": \"strict\""));
    // Python: ensure_ascii, lowercase hex, astral chars as surrogate pairs, DEL left raw.
    assert!(
        out.contains("\"branch\": \"q\\\"\\\\/\\n\\t\\u0001\u{7f}\\u00e9\\ud83d\\ude00\""),
        "{out}"
    );
}

#[test]
fn json_costs_are_rounded_to_four_places_and_printed_like_python_repr() {
    let mk = |cost: f64| {
        let mut s = Summary::default();
        s.by_model.push(("m".to_owned(), Bucket::default()));
        let c = Costs {
            by_model: vec![("m".to_owned(), Some(cost))],
            total: cost,
            unpriced_models: 0,
            ..Costs::default()
        };
        json(&s, &c)
    };
    // (input, python round(x, 4) repr)
    let cases = [
        (1.0, "1.0"),
        (0.00005, "0.0001"), // 5e-05 is just above the tie in binary
        (0.00004, "0.0"),
        (0.12345, "0.1235"), // binary value is 0.123450000000000004..., above the tie
        (0.12355, "0.1235"), // binary value is 0.12354999999999999..., below the tie
        (1_234_567.891_23, "1234567.8912"),
        (1e16, "1e+16"),
        (1.5e17, "1.5e+17"),
        (123_456_789_012.345_6, "123456789012.3456"),
    ];
    for (input, repr) in cases {
        let out = mk(input);
        assert!(
            out.contains(&format!("\"costUSD\": {repr}\n")),
            "{input}: {out}"
        );
        assert!(
            out.contains(&format!("\"totalCostUSD\": {repr},\n")),
            "{input}: {out}"
        );
    }
}

/// Re-serialises the real reference output in the `claim.json` goldens. The goldens are
/// key-sorted by the parity harness, so they are compared as parsed values, which still pins
/// every number, the session and model ordering, the empty-list/`null`/`0.0` cases and floats.
#[test]
fn json_reproduces_reference_claim_goldens() {
    use serde_json::Value;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/claude-code");
    let mut compared = 0;
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path().join("goldens/claim.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let all: Value = serde_json::from_str(&text).unwrap();
        for payload in all.as_object().unwrap().values() {
            if payload.get("error").is_some() {
                continue; // a branch with no records: the reference prints no report
            }
            let (summary, costs) = from_payload(payload);
            let mut ours: Value = serde_json::from_str(&json(&summary, &costs)).unwrap();
            // Deliberate divergences from the reference, #5: the floor flag and the price table
            // are new keys. Each must be present; every other key must match exactly.
            let object = ours.as_object_mut().unwrap();
            let unpriced = payload["models"]
                .as_object()
                .unwrap()
                .values()
                .any(|m| m["costUSD"].is_null());
            assert_eq!(
                object.remove("totalCostIsFloor"),
                Some(Value::Bool(unpriced))
            );
            assert!(object.remove("unpricedModels").is_some());
            assert!(object.remove("priceTable").is_some());
            assert_eq!(&ours, payload, "{}", path.display());
            compared += 1;
        }
    }
    assert!(compared >= 5, "compared only {compared} payloads");
}

/// Builds render inputs from a reference payload. Models go in the golden's key order (sorted
/// by name), which render must not depend on.
#[expect(
    clippy::unwrap_used,
    reason = "test helper reads a trusted golden and fails loudly"
)]
fn from_payload(p: &serde_json::Value) -> (Summary, Costs) {
    let u = |v: &serde_json::Value, k: &str| v[k].as_u64().unwrap();
    let mut summary = Summary {
        branch: p["branch"].as_str().unwrap().to_owned(),
        mode: if p["mode"] == "strict" {
            Mode::Strict
        } else {
            Mode::Claim
        },
        ..Summary::default()
    };
    summary.total.calls = u(p, "totalCalls");
    let mut costs = Costs {
        total: p["totalCostUSD"].as_f64().unwrap(),
        ..Costs::default()
    };
    for (name, m) in p["models"].as_object().unwrap() {
        summary.by_model.push((
            name.clone(),
            bucket(
                u(m, "input"),
                u(m, "cacheWrite5m"),
                u(m, "cacheWrite1h"),
                u(m, "cacheRead"),
                u(m, "output"),
                u(m, "thinking"),
                u(m, "calls"),
            ),
        ));
        let cost = m["costUSD"].as_f64();
        costs.unpriced_models += u64::from(cost.is_none());
        costs.by_model.push((name.clone(), cost));
    }
    for s in p["sessions"].as_array().unwrap() {
        let agents: Vec<&str> = s["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        summary.sessions.push((
            SessionId::new(s["id"].as_str().unwrap()),
            session(
                s["first"].as_str().unwrap(),
                s["last"].as_str().unwrap(),
                s["origin"].as_str().unwrap(),
                u(s, "tokens"),
                u(s, "calls"),
                &agents,
            ),
        ));
    }
    (summary, costs)
}

// ---- Subagent breakdown (#6) ----

use branch_tally_core::model::{AgentId, RowCost, SubagentRow, SubagentSummary, Workspace};

/// A row split by model, as Summary fills it: `tokens` and `calls` follow the bucket.
fn sub_row_m(ws: Workspace, name: &str, id: &str, b: Bucket) -> SubagentRow {
    SubagentRow {
        workspace: ws,
        subagent: name.to_owned(),
        model: "claude-a".to_owned(),
        calls: b.calls,
        tokens: b.tokens(),
        agent_ids: [AgentId::new(id)].into_iter().collect(),
        by_model: vec![("claude-a".to_owned(), b)],
    }
}

fn row_cost(known: f64, priced: u64, unpriced: u64) -> RowCost {
    RowCost {
        known,
        priced_models: priced,
        unpriced_models: unpriced,
    }
}

fn sub_row(ws: Workspace, name: &str, calls: u64, tokens: u64, ids: &[&str]) -> SubagentRow {
    SubagentRow {
        workspace: ws,
        subagent: name.to_owned(),
        model: String::new(),
        calls,
        tokens,
        agent_ids: ids.iter().map(|i| AgentId::new(*i)).collect(),
        by_model: Vec::new(),
    }
}

/// Main checkout plus two worktrees, one of which holds two subagents.
fn with_subagents(unattributed: (u64, u64)) -> (Summary, Costs) {
    with_subagents_unlinked(unattributed, (0, 0))
}

/// As [`with_subagents`], with `in_total` unlinked calls included in the branch total.
fn with_subagents_unlinked(unattributed: (u64, u64), in_total: (u64, u64)) -> (Summary, Costs) {
    let (mut s, mut c) = sample();
    s.subagents = SubagentSummary {
        rows: vec![
            sub_row_m(
                Workspace::Main,
                "Explore",
                "e1",
                bucket(1_000, 200, 100, 9_000, 1_700, 500, 4),
            ),
            sub_row_m(
                Workspace::worktree("agent-a1"),
                "Core Implementer",
                "a1",
                bucket(4_000, 60_000, 40_000, 300_000, 6_000, 2_000, 20),
            ),
            sub_row_m(
                Workspace::worktree("agent-a1"),
                "Reviewer",
                "a2",
                bucket(500, 0, 0, 4_000, 500, 0, 2),
            ),
            sub_row_m(
                Workspace::worktree("agent-b2"),
                "Harness Source Author",
                "b1",
                bucket(80_000, 0, 0, 200_000, 100_000, 10_000, 18),
            ),
        ],
        unattributed_calls: unattributed.0,
        unattributed_tokens: unattributed.1,
        unlinked_in_total_calls: in_total.0,
        unlinked_in_total_tokens: in_total.1,
    };
    // Row 0 priced, row 1 a floor, row 2 unpriced, row 3 has no entry (pricing never saw it).
    c.subagent_rows = vec![
        row_cost(0.5, 1, 0),
        row_cost(12.25, 1, 1),
        row_cost(0.0, 0, 1),
    ];
    c.subagent_total = row_cost(12.75, 2, 2);
    (s, c)
}

#[test]
fn markdown_lists_each_workspace_and_subagent_row_with_totals() {
    let (s, c) = with_subagents((0, 0));
    let md = markdown(&s, &c, true);
    // Priced, floor, unpriced, and no cost entry at all (last row).
    let want = "### Subagent usage (rolled up to `feat/x`)\n\
        \n\
        | Workspace | Subagent | Model | Calls | Input | Cache write | Cache read | Output | (thinking) \
        | Tokens | Est. cost |\n\
        |---|---|---|--:|--:|--:|--:|--:|--:|--:|--:|\n\
        | main checkout | `Explore` | `claude-a` | 4 | 1,000 | 300 | 9,000 | 1,700 | 500 | 12,000 | $0.50 |\n\
        | worktree `agent-a1` | `Core Implementer` | `claude-a` | 20 | 4,000 | 100,000 | 300,000 | 6,000 \
        | 2,000 | 410,000 | at least $12.25 |\n\
        | worktree `agent-a1` | `Reviewer` | `claude-a` | 2 | 500 | 0 | 4,000 | 500 | 0 | 5,000 | \u{2014} |\n\
        | worktree `agent-b2` | `Harness Source Author` | `claude-a` | 18 | 80,000 | 0 | 200,000 | 100,000 \
        | 10,000 | 380,000 | \u{2014} |\n\
        \n\
        807,000 tokens (44 calls) from 4 dispatched subagent(s) across 3 workspace(s) \
        (2 worktree(s)); est. cost at least $12.75.\n\
        0 subagent call(s) had no dispatch link.\n";
    assert!(md.contains(want), "{md}");
    // The section sits between the footnote and the attribution details.
    let at = md.find("### Subagent usage").unwrap();
    assert!(md.find("<sub>Branch").unwrap() < at);
    assert!(at < md.find("<details>").unwrap());
}

#[test]
fn markdown_reports_unattributed_subagent_calls_without_claiming_a_branch() {
    let (s, c) = with_subagents((3, 1_500));
    let md = markdown(&s, &c, false);
    assert!(
        md.contains(
            "3 subagent call(s) (1,500 tokens) across this repository's transcripts had no \
             dispatch link, so their branch is unverified (raw stamp kept); none of them are \
             included in this report's total.\n"
        ),
        "{md}"
    );
}

#[test]
fn markdown_says_how_many_unlinked_calls_are_inside_this_reports_total() {
    let (s, c) = with_subagents_unlinked((3, 1_500), (2, 900));
    let md = markdown(&s, &c, false);
    assert!(
        md.contains(
            "3 subagent call(s) (1,500 tokens) across this repository's transcripts had no \
             dispatch link, so their branch is unverified (raw stamp kept); 2 call(s) (900 \
             tokens) of them are included in this report's total via session claim.\n"
        ),
        "{md}"
    );
    // The headline line still counts only rolled-up rows.
    assert!(
        md.contains("807,000 tokens (44 calls) from 4 dispatched subagent(s)"),
        "{md}"
    );
}

#[test]
fn markdown_with_only_unattributed_calls_still_says_so() {
    let (mut s, c) = sample();
    s.subagents = SubagentSummary {
        unattributed_calls: 2,
        unattributed_tokens: 40,
        ..SubagentSummary::default()
    };
    let md = markdown(&s, &c, false);
    assert!(
        md.contains("### Subagent usage (rolled up to `feat/x`)\n"),
        "{md}"
    );
    assert!(!md.contains("| Workspace |"), "{md}");
    assert!(
        md.contains("2 subagent call(s) (40 tokens) across this repository's transcripts had no"),
        "{md}"
    );
}

#[test]
fn markdown_without_subagents_is_unchanged() {
    let (s, c) = sample();
    assert!(s.subagents.is_empty());
    assert!(!markdown(&s, &c, true).contains("Subagent usage"));
}

#[test]
fn markdown_escapes_pipes_in_subagent_cells() {
    let (mut s, c) = sample();
    s.subagents.rows = vec![sub_row(Workspace::Main, "a|b", 1, 1, &["x"])];
    let md = markdown(&s, &c, false);
    assert!(
        md.contains(
            "| main checkout | `a\\|b` | \u{2014} | 1 | \u{2014} | \u{2014} | \u{2014} | \u{2014} | \u{2014} | 1 |"
        ),
        "{md}"
    );
}

#[test]
fn json_has_a_subagents_object_with_rows_and_totals() {
    use serde_json::Value;
    let (s, c) = with_subagents_unlinked((3, 1_500), (2, 900));
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    let sub = &v["subagents"];
    assert_eq!(sub["unlinkedInTotalCalls"], 2);
    assert_eq!(sub["unlinkedInTotalTokens"], 900);
    assert_eq!(sub["calls"], 44);
    assert_eq!(sub["tokens"], 807_000);
    assert_eq!(sub["dispatched"], 4);
    assert_eq!(sub["workspaces"], 3);
    assert_eq!(sub["worktrees"], 2);
    assert_eq!(sub["unattributedCalls"], 3);
    assert_eq!(sub["unattributedTokens"], 1_500);
    let rows = sub["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["workspace"], "main");
    assert_eq!(rows[0]["isWorktree"], false);
    assert_eq!(rows[0]["subagent"], "Explore");
    assert_eq!(rows[0]["calls"], 4);
    assert_eq!(rows[0]["tokens"], 12_000);
    assert_eq!(rows[0]["agents"], 1);
    assert_eq!(rows[1]["workspace"], "agent-a1");
    assert_eq!(rows[1]["isWorktree"], true);
    assert_eq!(rows[2]["workspace"], "agent-a1");
}

#[test]
fn json_omits_subagents_when_there_is_nothing_to_report() {
    use serde_json::Value;
    let (s, c) = sample();
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v.get("subagents").is_none());
}

#[test]
fn json_keeps_subagents_for_unattributed_only() {
    use serde_json::Value;
    let (mut s, c) = sample();
    s.subagents.unattributed_calls = 2;
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["subagents"]["unattributedCalls"], 2);
    assert_eq!(v["subagents"]["rows"].as_array().unwrap().len(), 0);
}

#[test]
fn unknown_workspace_row_is_labelled_in_markdown_and_unambiguous_in_json() {
    use serde_json::Value;
    let (mut s, c) = sample();
    s.subagents.rows = vec![
        sub_row(Workspace::Unknown, "Explore", 2, 10, &["u1"]),
        sub_row(Workspace::worktree("unknown"), "Explore", 1, 5, &["u2"]),
    ];
    let md = markdown(&s, &c, false);
    assert!(
        md.contains(
            "| unknown workspace | `Explore` | \u{2014} | 2 | \u{2014} | \u{2014} | \u{2014} | \u{2014} | \u{2014} | 10 |"
        ),
        "{md}"
    );
    assert!(
        md.contains(
            "| worktree `unknown` | `Explore` | \u{2014} | 1 | \u{2014} | \u{2014} | \u{2014} | \u{2014} | \u{2014} | 5 |"
        ),
        "{md}"
    );
    assert!(!md.contains("| main checkout | `Explore`"), "{md}");
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    let rows = v["subagents"]["rows"].as_array().unwrap();
    assert_eq!(rows[0]["workspaceKind"], "unknown");
    assert_eq!(rows[0]["isWorktree"], false);
    assert_eq!(rows[1]["workspaceKind"], "worktree");
    assert_eq!(rows[1]["workspace"], "unknown");
    assert_eq!(v["subagents"]["workspaces"], 2);
    assert_eq!(v["subagents"]["worktrees"], 1);
}

// ---- Subagent cost and usage columns ----

#[test]
fn markdown_drops_the_subagent_cost_column_and_total_under_no_cost() {
    let (s, c) = with_subagents((0, 0));
    let md = markdown(&s, &c, false);
    let want = "| Workspace | Subagent | Model | Calls | Input | Cache write | Cache read | Output \
        | (thinking) | Tokens |\n\
        |---|---|---|--:|--:|--:|--:|--:|--:|--:|\n\
        | main checkout | `Explore` | `claude-a` | 4 | 1,000 | 300 | 9,000 | 1,700 | 500 | 12,000 |\n";
    assert!(md.contains(want), "{md}");
    let section = &md[md.find("### Subagent usage").unwrap()..md.find("<details>").unwrap()];
    assert!(!section.to_lowercase().contains("est. cost"), "{section}");
    assert!(!section.contains('$'), "{section}");
    assert!(section.contains("(2 worktree(s)).\n"), "{section}");
}

#[test]
fn markdown_shows_a_dash_for_the_section_total_when_nothing_was_priced() {
    let (s, mut c) = with_subagents((0, 0));
    c.subagent_total = row_cost(0.0, 0, 2);
    let md = markdown(&s, &c, true);
    assert!(
        md.contains("(2 worktree(s)); est. cost \u{2014}.\n"),
        "{md}"
    );
}

#[test]
fn markdown_section_total_is_exact_when_every_row_was_priced() {
    let (s, mut c) = with_subagents((0, 0));
    c.subagent_total = row_cost(1.5, 4, 0);
    let md = markdown(&s, &c, true);
    assert!(md.contains("(2 worktree(s)); est. cost $1.50.\n"), "{md}");
}

#[test]
fn markdown_without_a_model_split_shows_dashes_not_zero_usage() {
    let (mut s, c) = sample();
    s.subagents.rows = vec![sub_row(Workspace::Main, "Old", 3, 99, &["o1"])];
    let md = markdown(&s, &c, true);
    assert!(
        md.contains(
            "| main checkout | `Old` | \u{2014} | 3 | \u{2014} | \u{2014} | \u{2014} | \u{2014} | \u{2014} \
             | 99 | \u{2014} |\n"
        ),
        "{md}"
    );
}

#[test]
fn json_subagent_rows_carry_usage_split_and_cost() {
    use serde_json::Value;
    let (s, c) = with_subagents((0, 0));
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    let rows = v["subagents"]["rows"].as_array().unwrap();
    let r = &rows[0];
    assert_eq!(r["input"], 1_000);
    assert_eq!(r["cacheWrite5m"], 200);
    assert_eq!(r["cacheWrite1h"], 100);
    assert_eq!(r["cacheRead"], 9_000);
    assert_eq!(r["output"], 1_700);
    assert_eq!(r["thinking"], 500);
    assert_eq!(r["costUSD"], 0.5);
    assert_eq!(r["costIsFloor"], false);
    // Existing keys stay.
    assert_eq!(r["tokens"], 12_000);
    assert_eq!(r["agents"], 1);
    // Floor: a number that is a lower bound.
    assert_eq!(rows[1]["costUSD"], 12.25);
    assert_eq!(rows[1]["costIsFloor"], true);
    // Unpriced and absent: null, not zero.
    assert!(rows[2]["costUSD"].is_null());
    assert_eq!(rows[2]["costIsFloor"], false);
    assert!(rows[3]["costUSD"].is_null());
    assert_eq!(rows[3]["costIsFloor"], false);
    // Section cost.
    assert_eq!(v["subagents"]["costUSD"], 12.75);
    assert_eq!(v["subagents"]["costIsFloor"], true);
}

#[test]
fn json_subagent_section_cost_is_null_when_unpriced_and_split_null_without_models() {
    use serde_json::Value;
    let (mut s, c) = sample();
    s.subagents.rows = vec![sub_row(Workspace::Main, "Old", 3, 99, &["o1"])];
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v["subagents"]["costUSD"].is_null());
    assert_eq!(v["subagents"]["costIsFloor"], false);
    let r = &v["subagents"]["rows"][0];
    assert!(r["input"].is_null());
    assert!(r["thinking"].is_null());
    assert!(r["costUSD"].is_null());
    assert_eq!(r["tokens"], 99);
}

#[test]
fn markdown_subagent_model_column_follows_subagent_and_survives_no_cost() {
    let (s, c) = with_subagents((0, 0));
    let md = markdown(&s, &c, false);
    assert!(
        md.contains(
            "| Workspace | Subagent | Model | Calls | Input | Cache write | Cache read | Output \
             | (thinking) | Tokens |\n|---|---|---|--:|--:|--:|--:|--:|--:|--:|\n"
        ),
        "{md}"
    );
    assert!(
        md.contains("| main checkout | `Explore` | `claude-a` | 4 | 1,000 |"),
        "{md}"
    );
}

#[test]
fn a_subagent_with_two_models_is_two_rows_each_with_its_own_model_and_cost() {
    let (mut s, mut c) = sample();
    let mut first = sub_row_m(
        Workspace::Main,
        "Explore",
        "e1",
        bucket(1, 0, 0, 0, 1, 0, 1),
    );
    let mut second = sub_row_m(
        Workspace::Main,
        "Explore",
        "e1",
        bucket(2, 0, 0, 0, 2, 0, 2),
    );
    first.model = "claude-a".to_owned();
    second.model = "claude-b".to_owned();
    second.by_model = vec![("claude-b".to_owned(), bucket(2, 0, 0, 0, 2, 0, 2))];
    s.subagents = SubagentSummary {
        rows: vec![first, second],
        ..SubagentSummary::default()
    };
    c.subagent_rows = vec![row_cost(1.0, 1, 0), row_cost(2.0, 1, 0)];
    c.subagent_total = row_cost(3.0, 2, 0);
    let md = markdown(&s, &c, true);
    assert!(
        md.contains("| main checkout | `Explore` | `claude-a` | 1 |"),
        "{md}"
    );
    assert!(
        md.contains("| main checkout | `Explore` | `claude-b` | 2 |"),
        "{md}"
    );
    assert!(md.contains("from 1 dispatched subagent(s)"), "{md}");
    let v: serde_json::Value = serde_json::from_str(&json(&s, &c)).unwrap();
    let rows = v["subagents"]["rows"].as_array().unwrap();
    assert_eq!(rows[0]["model"], "claude-a");
    assert_eq!(rows[1]["model"], "claude-b");
    assert_eq!(v["subagents"]["dispatched"], 1);
}

#[test]
fn a_row_from_before_the_model_field_shows_a_dash_and_null_in_json() {
    let (mut s, c) = sample();
    s.subagents.rows = vec![sub_row(Workspace::Main, "old", 1, 1, &["x"])];
    assert!(markdown(&s, &c, false).contains("| main checkout | `old` | \u{2014} | 1 |"));
    let v: serde_json::Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v["subagents"]["rows"][0]["model"].is_null());
}

fn hc(calls: u64, tokens: u64) -> HeuristicCount {
    HeuristicCount { calls, tokens }
}

fn footnote_line(md: &str) -> &str {
    md.lines().find(|l| l.starts_with("<sub>")).unwrap_or("")
}

#[test]
fn footnote_reports_calls_and_tokens_per_heuristic_used_and_omits_zero_ones() {
    let (mut s, c) = sample();
    s.footnote.heuristics = HeuristicBreakdown {
        session_claim: hc(2, 1_500),
        worktree_recovery: hc(1, 40),
        ..HeuristicBreakdown::default()
    };
    let md = markdown(&s, &c, false);
    assert_eq!(
        footnote_line(&md),
        "<sub>Branch `feat/x` \u{b7} 2 session(s), 1 in a worktree \u{b7} 1,234 of 3 calls from \
         subagents \u{b7} attributed by heuristic: session claim 2 call(s) / 1,500 tokens; \
         worktree recovery 1 call(s) / 40 tokens</sub>"
    );
    assert!(!md.contains("recovered from pre-branch"));
    assert!(!md.contains("origin recovery"));
    assert!(
        !md.contains("at least"),
        "no ambiguity, so no floor wording"
    );
}

#[test]
fn timeline_mode_is_named_and_ambiguity_makes_tokens_and_cost_a_floor() {
    let (mut s, mut c) = sample();
    s.mode = Mode::Timeline;
    s.footnote.heuristics = HeuristicBreakdown {
        timeline_forward: hc(1, 100),
        timeline_carried: hc(1, 200),
        dispatcher_roll_up: hc(3, 300),
        origin_recovery: hc(1, 5),
        ambiguous: hc(2, 7_000),
        ..HeuristicBreakdown::default()
    };
    c.unpriced_models = 0; // the only floor left is the ambiguity
    let md = markdown(&s, &c, true);
    let note = footnote_line(&md);
    assert!(
        note.starts_with("<sub>Branch `feat/x` \u{b7} timeline mode \u{b7} "),
        "{note}"
    );
    assert!(
        note.contains(
            "attributed by heuristic: origin recovery 1 call(s) / 5 tokens; dispatcher roll-up \
             3 call(s) / 300 tokens; timeline forward 1 call(s) / 100 tokens; timeline \
             carried 1 call(s) / 200 tokens"
        ),
        "{note}"
    );
    assert!(
        md.contains(
            "**Token total is at least 4,655**: 7,000 token(s) in 2 call(s) could not be \
             attributed to a branch with certainty."
        ),
        "{md}"
    );
    assert!(md.contains("**at least $2.67**"), "{md}");
    assert!(markdown(&s, &c, false).contains("Token total is at least"));
}

#[test]
fn json_carries_heuristics_and_floor_only_when_used() {
    use serde_json::Value;
    let (mut s, c) = sample();
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v.get("heuristics").is_none());
    s.footnote.heuristics = HeuristicBreakdown {
        session_claim: hc(2, 1_500),
        ambiguous: hc(1, 9),
        ..HeuristicBreakdown::default()
    };
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["heuristics"]["sessionClaim"]["calls"], 2);
    assert_eq!(v["heuristics"]["sessionClaim"]["tokens"], 1_500);
    assert_eq!(v["heuristics"]["timelineCarried"]["calls"], 0);
    assert_eq!(v["heuristics"]["ambiguous"]["tokens"], 9);
    assert_eq!(v["heuristics"]["isFloor"], true);
    assert_eq!(v["totalCostIsFloor"], true);
}

fn timeline_summary(rule: Option<branch_tally_core::model::WorkBranchRule>) -> (Summary, Costs) {
    let (mut s, c) = sample();
    s.mode = Mode::Timeline;
    s.work_branch = rule;
    (s, c)
}

#[test]
fn the_default_branch_is_named_when_it_did_not_come_from_origin_head() {
    use branch_tally_core::model::{DefaultBranchSource as Src, WorkBranchRule as Rule};
    let rule = |source| {
        Some(Rule::ExcludesDefault {
            branch: "develop".to_owned(),
            source,
        })
    };

    let (s, c) = timeline_summary(rule(Src::InitDefaultBranch));
    let md = markdown(&s, &c, false);
    assert!(
        footnote_line(&md).contains(
            " \u{b7} work branches: all except default branch `develop` (from \
             init.defaultBranch; origin/HEAD was not available)"
        ),
        "{md}"
    );

    let (s, c) = timeline_summary(rule(Src::AssumedMain));
    let md = markdown(&s, &c, false);
    assert!(
        footnote_line(&md).contains(
            " \u{b7} work branches: all except default branch `develop` (assumed; neither \
             origin/HEAD nor init.defaultBranch was available)"
        ),
        "{md}"
    );

    // From origin/HEAD it is the ordinary case: nothing extra to say in the footnote.
    let (s, c) = timeline_summary(rule(Src::OriginHead));
    assert!(!markdown(&s, &c, false).contains("work branches:"));
}

#[test]
fn a_custom_work_branch_pattern_is_stated() {
    use branch_tally_core::model::WorkBranchRule as Rule;
    let (s, c) = timeline_summary(Some(Rule::Custom {
        pattern: "^feat/".to_owned(),
    }));
    let md = markdown(&s, &c, false);
    assert!(
        footnote_line(&md).contains(" \u{b7} work branches: custom pattern `^feat/`"),
        "{md}"
    );
}

#[test]
fn json_states_the_work_branch_rule_only_when_there_is_one() {
    use branch_tally_core::model::{DefaultBranchSource as Src, WorkBranchRule as Rule};
    use serde_json::Value;
    let (s, c) = sample();
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v.get("workBranch").is_none());

    let (s, c) = timeline_summary(Some(Rule::ExcludesDefault {
        branch: "develop".to_owned(),
        source: Src::AssumedMain,
    }));
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["workBranch"]["kind"], "excludesDefault");
    assert_eq!(v["workBranch"]["defaultBranch"], "develop");
    assert_eq!(v["workBranch"]["source"], "assumed");

    let (s, c) = timeline_summary(Some(Rule::Custom {
        pattern: "^f\"x".to_owned(),
    }));
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["workBranch"]["kind"], "custom");
    assert_eq!(v["workBranch"]["pattern"], "^f\"x");
}

// ---- redacted output (#9) -------------------------------------------------------------------
// `redact` is a core stage that runs before render; these summaries are built by hand the way it
// leaves them: generic ids and names, dates instead of timestamps, `redacted` set.

fn redacted_sample() -> (Summary, Costs) {
    let (mut s, c) = sample();
    s.redacted = true;
    s.sessions = vec![
        (
            SessionId::new("s1"),
            session(
                "2026-01-01",
                "2026-01-01",
                "worktree-1",
                50_000,
                1,
                &["agent-1"],
            ),
        ),
        (
            SessionId::new("s2"),
            session("2026-01-02", "2026-01-03", "main", 1_234_567, 2, &[]),
        ),
        (SessionId::new("s3"), session("", "", "main", 10, 1, &[])),
    ];
    (s, c)
}

#[test]
fn redacted_markdown_with_cost_is_pinned() {
    let (s, c) = redacted_sample();
    insta::assert_snapshot!("markdown_redacted_cost", markdown(&s, &c, true));
}

#[test]
fn redacted_markdown_without_cost_is_pinned() {
    let (s, c) = redacted_sample();
    insta::assert_snapshot!("markdown_redacted_no_cost", markdown(&s, &c, false));
}

#[test]
fn redacted_json_is_pinned() {
    let (s, c) = redacted_sample();
    insta::assert_snapshot!("json_redacted", json(&s, &c));
}

#[test]
fn redacted_output_says_so_and_prints_dates_whole() {
    let (s, c) = redacted_sample();
    for show_cost in [true, false] {
        let md = markdown(&s, &c, show_cost);
        let note = footnote_line(&md);
        assert!(note.contains(" \u{b7} identifiers redacted"), "{note}");
        assert!(md.contains("| Span (UTC date) |"), "{md}");
        assert!(md.contains("| 2026-01-02 \u{2192} 2026-01-03 |"), "{md}");
        // An empty date stays empty, as the clock span does today.
        assert!(md.contains("|  \u{2192}  |"), "{md}");
    }
    assert!(json(&s, &c).contains("\n  \"redacted\": true,\n"));
}

#[test]
fn unredacted_json_has_no_redacted_key() {
    let (s, c) = sample();
    assert!(!json(&s, &c).contains("redacted"));
}

#[test]
fn unredacted_markdown_has_no_redaction_note_and_keeps_the_clock_span() {
    let (s, c) = sample();
    let md = markdown(&s, &c, true);
    assert!(!md.contains("redacted"));
    assert!(md.contains("| Span (UTC) |"));
}

// ---- escaping transcript-derived strings (#39) ----------------------------------------------
// Model ids, the branch, session ids, agent, attribution, worktree and subagent names come from
// transcripts. A `|`, a backtick, `<`/`>`, `&`, a backslash before a pipe or a newline in any of
// them must not end a cell, open a code span or put raw HTML into the PR body.

/// The cells of a GFM table row as cmark-gfm splits them: a backslash escapes the next character,
/// so only an unescaped `|` is a delimiter. Returns the number of delimiters.
fn unescaped_pipes(line: &str) -> usize {
    let mut count = 0;
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '|' => count += 1,
            _ => {}
        }
    }
    count
}

fn hostile_sample() -> (Summary, Costs) {
    let (mut s, mut c) = sample();
    "feat/`x`|<b>".clone_into(&mut s.branch);
    s.by_model = vec![
        ("m|pipe".to_owned(), bucket(1, 0, 0, 0, 50, 0, 1)),
        ("`tick".to_owned(), bucket(1, 0, 0, 0, 40, 0, 1)),
        ("m<b>&".to_owned(), bucket(1, 0, 0, 0, 30, 0, 1)),
        (r"m\|bs".to_owned(), bucket(1, 0, 0, 0, 20, 0, 1)),
        ("m\nnl".to_owned(), bucket(1, 0, 0, 0, 10, 0, 1)),
        (
            "m\u{1b}[31m\u{202e}rtl".to_owned(),
            bucket(1, 0, 0, 0, 5, 0, 1),
        ),
    ];
    c.by_model = s.by_model.iter().map(|(m, _)| (m.clone(), None)).collect();
    s.by_attribution = vec![
        ("agent:<x>|y".to_owned(), bucket(0, 0, 0, 0, 30, 0, 1)),
        ("skill:`z`".to_owned(), bucket(0, 0, 0, 0, 20, 0, 1)),
        (r"back\slash|".to_owned(), bucket(0, 0, 0, 0, 10, 0, 1)),
        (
            "[k](http://e.co) ![](http://i.co/x.png) http://bare.co @user #123 *em*".to_owned(),
            bucket(0, 0, 0, 0, 5, 0, 1),
        ),
        (
            "skill:\u{1b}[2J\u{202e}".to_owned(),
            bucket(0, 0, 0, 0, 4, 0, 1),
        ),
    ];
    s.sessions = vec![(
        SessionId::new("ab|`<x"),
        session(
            "2026-01-01T10:00:00Z",
            "2026-01-01T10:30:00Z",
            "main",
            10,
            1,
            &["a|b", "`c`", "<d>", "e&f"],
        ),
    )];
    s.subagents = SubagentSummary {
        rows: vec![sub_row(
            Workspace::worktree("`wt|"),
            "<img src=x>|`",
            1,
            1,
            &["x"],
        )],
        ..SubagentSummary::default()
    };
    (s, c)
}

#[test]
fn every_table_row_keeps_its_column_count_whatever_the_names_contain() {
    let (s, c) = hostile_sample();
    for show_cost in [true, false] {
        let md = markdown(&s, &c, show_cost);
        let mut columns = None;
        for line in md.lines() {
            if !line.starts_with('|') {
                columns = None;
                continue;
            }
            let pipes = unescaped_pipes(line);
            let expected = *columns.get_or_insert(pipes);
            assert_eq!(
                pipes, expected,
                "row splits differently from its header: {line}\n{md}"
            );
        }
    }
}

#[test]
fn code_span_cells_escape_pipes_and_fence_past_backticks() {
    let (s, c) = hostile_sample();
    let md = markdown(&s, &c, false);
    for cell in [
        r"| `m\|pipe` |",
        "| `` `tick `` |",
        "| `m<b>&` |",
        r"| `m\\\|bs` |",
        "| `m nl` |",
        r"| ``ab\|`<x`` |",
        r"| worktree `` `wt\| `` |",
    ] {
        assert!(md.contains(cell), "missing {cell}\n{md}");
    }
}

#[test]
fn name_cells_are_code_spans_so_markdown_and_html_stay_literal() {
    let (s, c) = hostile_sample();
    let md = markdown(&s, &c, false);
    for cell in [
        r"| `<d>`, `` `c` ``, `a\|b`, `e&f` |",
        r"| `agent:<x>\|y` |",
        r"| `` skill:`z` `` |",
        r"| `back\slash\|` |",
        r"| `` <img src=x>\|` `` |",
        "| `[k](http://e.co) ![](http://i.co/x.png) http://bare.co @user #123 *em*` |",
    ] {
        assert!(md.contains(cell), "missing {cell}\n{md}");
    }
    // No name cell is left as live text: the only angle brackets are inside code spans.
    assert!(!md.contains("&lt;") && !md.contains("&amp;"), "{md}");
}

#[test]
fn control_characters_never_reach_markdown() {
    let (s, c) = hostile_sample();
    for show_cost in [true, false] {
        let md = markdown(&s, &c, show_cost);
        let bad: Vec<_> = md
            .chars()
            .filter(|&ch| ch != '\n' && branch_tally_core::text::is_unsafe_control(ch))
            .collect();
        assert!(bad.is_empty(), "{bad:?}\n{md}");
    }
    let md = markdown(&s, &c, false);
    assert!(md.contains("`m\u{FFFD}[31m\u{FFFD}rtl`"), "{md}");
    assert!(md.contains("`skill:\u{FFFD}[2J\u{FFFD}`"), "{md}");
}

#[test]
fn a_hostile_session_span_is_filtered_to_the_timestamp_alphabet() {
    let (mut s, c) = hostile_sample();
    s.sessions[0].1.first = "2026-01-01T10:00:00Z|[a](//x.co)@u<img>\u{1b}".to_owned();
    let md = markdown(&s, &c, false);
    let line = md
        .lines()
        .find(|l| l.contains("main checkout"))
        .unwrap_or("");
    assert!(
        !line.contains("](") && !line.contains('@') && !line.contains("<img"),
        "{md}"
    );
}

#[test]
fn the_branch_is_a_code_span_in_the_footnote_and_the_subagent_heading() {
    let (s, c) = hostile_sample();
    let md = markdown(&s, &c, false);
    assert!(md.contains("<sub>Branch ``feat/`x`|<b>`` \u{b7}"), "{md}");
    assert!(
        md.contains("### Subagent usage (rolled up to ``feat/`x`|<b>``)"),
        "{md}"
    );
}

#[test]
fn hostile_names_markdown_is_pinned() {
    let (s, c) = hostile_sample();
    insta::assert_snapshot!("markdown_hostile_names_cost", markdown(&s, &c, true));
    insta::assert_snapshot!("markdown_hostile_names_no_cost", markdown(&s, &c, false));
}

#[test]
fn footnote_names_captured_hook_data_only_when_used() {
    let (mut s, c) = sample();
    assert!(!markdown(&s, &c, false).contains("captured"));
    s.footnote.heuristics = HeuristicBreakdown {
        captured: hc(4, 800),
        ..HeuristicBreakdown::default()
    };
    let md = markdown(&s, &c, false);
    assert!(
        footnote_line(&md)
            .contains("attributed by heuristic: captured hook data 4 call(s) / 800 tokens"),
        "{md}"
    );
}

#[test]
fn json_carries_captured_only_when_used() {
    use serde_json::Value;
    let (mut s, c) = sample();
    s.footnote.heuristics = HeuristicBreakdown {
        session_claim: hc(2, 10),
        ..HeuristicBreakdown::default()
    };
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(
        v["heuristics"].get("captured").is_none(),
        "zero stays absent"
    );
    s.footnote.heuristics = HeuristicBreakdown {
        captured: hc(4, 800),
        ..HeuristicBreakdown::default()
    };
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["heuristics"]["captured"]["calls"], 4);
    assert_eq!(v["heuristics"]["captured"]["tokens"], 800);
}

#[test]
fn markdown_reports_unstamped_calls_in_the_footer_only_when_there_are_some() {
    let (mut s, c) = sample();
    assert!(!markdown(&s, &c, false).contains("no branch stamp"));
    s.unstamped = Unstamped {
        calls: 2,
        tokens: 2_221,
    };
    let md = markdown(&s, &c, false);
    let line = "2 call(s) / 2,221 tokens in this repository carry no branch stamp and are not \
                attributed to any branch";
    assert!(md.contains(line), "{md}");
    // It sits after the footnote, before the attribution details.
    assert!(md.find("<sub>Branch").unwrap() < md.find(line).unwrap());
    assert!(md.find(line).unwrap() < md.find("<details>").unwrap());
}

#[test]
fn json_has_a_top_level_unstamped_key_only_when_there_are_some() {
    use serde_json::Value;
    let (mut s, c) = sample();
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert!(v.get("unstamped").is_none());
    s.unstamped = Unstamped {
        calls: 2,
        tokens: 2_221,
    };
    let v: Value = serde_json::from_str(&json(&s, &c)).unwrap();
    assert_eq!(v["unstamped"]["calls"], 2);
    assert_eq!(v["unstamped"]["tokens"], 2_221);
}

#[test]
fn splice_uses_the_last_end_after_the_first_start() {
    let body = format!("a\n{START}\nold {END} tail {END}\nz");
    assert_eq!(splice(&body, "n"), format!("a\n{START}\nn\n{END}\nz"));
}

#[test]
fn splice_ignores_an_end_before_the_start() {
    let body = format!("{END} head\n{START}\nold\n{END}\nz");
    assert_eq!(
        splice(&body, "n"),
        format!("{END} head\n{START}\nn\n{END}\nz")
    );
}

#[test]
fn splicing_a_hostile_report_repeatedly_is_idempotent() {
    let (mut s, c) = hostile_sample();
    s.by_model[0].0 = format!("m{END}tail{START}x");
    s.branch = format!("b{END}");
    let report = markdown(&s, &c, true);
    let once = splice("pr body", &report);
    let twice = splice(&once, &report);
    let thrice = splice(&twice, &report);
    assert_eq!(once, twice);
    assert_eq!(twice, thrice);
}
