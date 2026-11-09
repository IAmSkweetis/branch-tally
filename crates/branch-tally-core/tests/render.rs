//! Byte-exact checks of `render` against `reference/token_report.py`'s `render`, `wrap`, `splice`
//! and `as_json`, from hand-built precomputed inputs. Expected strings were derived by hand from
//! the reference's format strings, not copied from this crate's output.

use std::collections::BTreeSet;

use branch_tally_core::model::{
    Bucket, Costs, Footnote, Mode, PriceTableInfo, PriceTableSource, SessionId, SessionRow, Summary,
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
         | `s2` | worktree | a, x | 01-01 10:00 \u{2192} 01-01 10:30 | 1 | 0.1M |\n\
         | `abcdefgh` | main checkout | \u{2014} | 01-02 03:04 \u{2192} 01-02 05:06 | 2 | 1.2M |\n\
         \n\
         <sub>Branch `feat/x` \u{b7} 2 session(s), 1 in a worktree \u{b7} 1,234 of 3 calls from \
         subagents \u{b7} 2.5M tokens recovered from pre-branch/worktree checkouts{note_end}\n\
         \n\
         <details><summary>Breakdown by skill / agent</summary>\n\
         \n\
         | Attribution | Calls | Tokens |\n\
         |---|--:|--:|\n\
         | agent:x | 1 | 90 |\n\
         | main | 2 | 10 |\n\
         | skill | 1 | 10 |\n\
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
