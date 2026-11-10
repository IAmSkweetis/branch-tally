//! The machine-readable report, formatted as Python's `json.dumps(payload, indent=2)` does.
//!
//! The emitter is hand-written, not `serde_json`, for three reasons that all come down to byte
//! parity with the reference: key order is insertion order (`serde_json` sorts keys unless the
//! `preserve_order` feature is on, which would need a manifest change), `ensure_ascii` escapes
//! every non-ASCII char (`serde_json` emits UTF-8), and floats print as Python's `repr` after
//! `round(x, 4)` (`1e-05`, `1e+16`, `0.0`), which `serde_json` spells differently.

use std::cmp::Reverse;
use std::fmt::Write as _;

use crate::model::{
    Costs, HeuristicBreakdown, PriceTableSource, RowCost, SubagentSummary, Summary, WorkBranchRule,
};

/// Python `json.dumps` string escaping with `ensure_ascii=True`.
fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='\u{7f}' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    // Writing to a String cannot fail.
                    let _ = write!(out, "\\u{u:04x}");
                }
            }
        }
    }
    out.push('"');
    out
}

/// `repr(round(x, 4))` as Python prints it inside JSON (`NaN` and `Infinity` as `json.dumps`
/// writes them).
///
/// `round(x, 4)` is correctly rounded on the exact binary value, ties to even. Formatting with
/// `{:.4}` does the same in Rust, and parsing the text back gives the nearest double, which is
/// what Python's `round` returns. The shortest round-trip digits then come from `{:e}`.
fn float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let rounded = format!("{x:.4}").parse::<f64>().unwrap_or(x);
    repr(rounded)
}

/// Python's `float.__repr__` for a finite value.
fn repr(x: f64) -> String {
    let sci = format!("{x:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if digits.chars().all(|c| c == '0') {
        return format!("{sign}0.0");
    }
    let (first, rest) = digits.split_at(1);
    if !(-4..16).contains(&exp) {
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exp < 0 { '-' } else { '+' };
        return format!("{sign}{first}{frac}e{exp_sign}{:02}", exp.abs());
    }
    if exp < 0 {
        let zeros = "0".repeat(usize::try_from(-exp - 1).unwrap_or(0));
        return format!("{sign}0.{zeros}{digits}");
    }
    let int_len = usize::try_from(exp).unwrap_or(0) + 1;
    if digits.len() <= int_len {
        let pad = "0".repeat(int_len - digits.len());
        format!("{sign}{digits}{pad}.0")
    } else {
        format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
    }
}

/// A subagent cost as JSON: the number (`null` when nothing was priced or there is no entry) and
/// whether it is only a floor.
fn row_cost(cost: Option<&RowCost>) -> (String, bool) {
    match cost {
        Some(c) if !c.is_unpriced() => (float(c.known), c.is_floor()),
        _ => ("null".to_owned(), false),
    }
}

/// The `priceTable` object: the table date (null when unknown) and where it came from.
fn price_table(costs: &Costs, o: &mut String) {
    let table = &costs.price_table;
    let as_of = if table.as_of.is_empty() {
        "null".to_owned()
    } else {
        string(&table.as_of)
    };
    let source = match table.source {
        PriceTableSource::Bundled => "bundled",
        PriceTableSource::Cache => "cache",
        PriceTableSource::Override => "override",
    };
    o.push_str("  \"priceTable\": {\n");
    let _ = writeln!(o, "    \"asOf\": {as_of},");
    let _ = writeln!(o, "    \"source\": \"{source}\"");
    o.push_str("  },\n");
}

/// The `heuristics` object (#7): calls and tokens per attribution heuristic, and whether the
/// total is only a floor because some calls were ambiguous. Omitted when no call was counted, so
/// reports that predate it keep their shape.
fn heuristics(h: &HeuristicBreakdown, o: &mut String) {
    let entries = [
        ("sessionClaim", h.session_claim),
        ("worktreeRecovery", h.worktree_recovery),
        ("originRecovery", h.origin_recovery),
        ("dispatcherRollUp", h.dispatcher_roll_up),
        ("timelineForward", h.timeline_forward),
        ("timelineCarried", h.timeline_carried),
        ("ambiguous", h.ambiguous),
    ];
    if entries.iter().all(|(_, c)| c.calls == 0) {
        return;
    }
    o.push_str("  \"heuristics\": {\n");
    for (key, c) in entries {
        let _ = writeln!(
            o,
            "    \"{key}\": {{ \"calls\": {}, \"tokens\": {} }},",
            c.calls, c.tokens
        );
    }
    let _ = writeln!(o, "    \"isFloor\": {}", h.any_ambiguous());
    o.push_str("  },\n");
}

/// The `workBranch` object (#7): which branches timeline mode treated as work branches. Present
/// only when the summary records a rule, i.e. in timeline mode.
fn work_branch(rule: Option<&WorkBranchRule>, o: &mut String) {
    match rule {
        None => {}
        Some(WorkBranchRule::ExcludesDefault { branch, source }) => {
            o.push_str("  \"workBranch\": {\n");
            o.push_str("    \"kind\": \"excludesDefault\",\n");
            let _ = writeln!(o, "    \"defaultBranch\": {},", string(branch));
            let _ = writeln!(o, "    \"source\": {}", string(source.as_str()));
            o.push_str("  },\n");
        }
        Some(WorkBranchRule::Custom { pattern }) => {
            o.push_str("  \"workBranch\": {\n");
            o.push_str("    \"kind\": \"custom\",\n");
            let _ = writeln!(o, "    \"pattern\": {}", string(pattern));
            o.push_str("  },\n");
        }
    }
}

/// The `subagents` object (#6). Not in the reference: omitted when empty, so strict mode and
/// reports without dispatched subagents keep the reference's shape. Totals come from the
/// accessors and count rolled-up records only; `unattributed*` span every attributed record, not
/// only this branch's; `unlinkedInTotal*` are the unlinked calls inside this report's total.
fn subagents(sub: &SubagentSummary, costs: &Costs, o: &mut String) {
    if sub.is_empty() {
        return;
    }
    o.push_str("  \"subagents\": {\n");
    let _ = writeln!(o, "    \"calls\": {},", sub.calls());
    let _ = writeln!(o, "    \"tokens\": {},", sub.tokens());
    let _ = writeln!(o, "    \"dispatched\": {},", sub.dispatched());
    let _ = writeln!(o, "    \"workspaces\": {},", sub.workspaces());
    let _ = writeln!(o, "    \"worktrees\": {},", sub.worktrees());
    let _ = writeln!(o, "    \"unattributedCalls\": {},", sub.unattributed_calls);
    let _ = writeln!(
        o,
        "    \"unattributedTokens\": {},",
        sub.unattributed_tokens
    );
    let _ = writeln!(
        o,
        "    \"unlinkedInTotalCalls\": {},",
        sub.unlinked_in_total_calls
    );
    let _ = writeln!(
        o,
        "    \"unlinkedInTotalTokens\": {},",
        sub.unlinked_in_total_tokens
    );
    let (cost, floor) = row_cost(Some(&costs.subagent_total));
    let _ = writeln!(o, "    \"costUSD\": {cost},");
    let _ = writeln!(o, "    \"costIsFloor\": {floor},");
    if sub.rows.is_empty() {
        o.push_str("    \"rows\": []\n");
    } else {
        o.push_str("    \"rows\": [\n");
        for (i, r) in sub.rows.iter().enumerate() {
            o.push_str("      {\n");
            let _ = writeln!(
                o,
                "        \"workspace\": {},",
                string(&r.workspace.label())
            );
            let _ = writeln!(
                o,
                "        \"workspaceKind\": {},",
                string(r.workspace.kind())
            );
            let _ = writeln!(o, "        \"isWorktree\": {},", r.workspace.is_worktree());
            let _ = writeln!(o, "        \"subagent\": {},", string(&r.subagent));
            // Empty only on a row from before rows carried a model: unknown, so null.
            let model = if r.model.is_empty() {
                "null".to_owned()
            } else {
                string(&r.model)
            };
            let _ = writeln!(o, "        \"model\": {model},");
            let _ = writeln!(o, "        \"calls\": {},", r.calls);
            let _ = writeln!(o, "        \"tokens\": {},", r.tokens);
            let _ = writeln!(o, "        \"agents\": {},", r.agent_ids.len());
            // Without a per-model split (older data) the usage is unknown, not zero.
            let t = r.total();
            for (key, v) in [
                ("input", t.input),
                ("cacheWrite5m", t.cache_write_5m),
                ("cacheWrite1h", t.cache_write_1h),
                ("cacheRead", t.cache_read),
                ("output", t.output),
                ("thinking", t.thinking),
            ] {
                let v = if r.by_model.is_empty() {
                    "null".to_owned()
                } else {
                    v.to_string()
                };
                let _ = writeln!(o, "        \"{key}\": {v},");
            }
            let (cost, floor) = row_cost(costs.subagent_row(i));
            let _ = writeln!(o, "        \"costUSD\": {cost},");
            let _ = writeln!(o, "        \"costIsFloor\": {floor}");
            o.push_str(if i + 1 == sub.rows.len() {
                "      }\n"
            } else {
                "      },\n"
            });
        }
        o.push_str("    ]\n");
    }
    o.push_str("  },\n");
}

/// The report as JSON (`as_json` in the reference).
///
/// `totalCalls` is `Summary::total.calls` (the reference's `len(mine)`, the same number as the
/// sum of per-model calls) and `totalCostUSD` is `Costs::total`. Sessions are sorted by `first`
/// and models by descending output, both stably.
#[must_use]
pub fn json(summary: &Summary, costs: &Costs) -> String {
    let mut sessions: Vec<_> = summary.sessions.iter().collect();
    sessions.sort_by(|a, b| a.1.first.cmp(&b.1.first));
    let mut models: Vec<_> = summary.by_model.iter().collect();
    models.sort_by_key(|(_, b)| Reverse(b.output));

    let mut o = String::new();
    o.push_str("{\n");
    let _ = writeln!(o, "  \"branch\": {},", string(&summary.branch));
    let _ = writeln!(o, "  \"mode\": {},", string(summary.mode.as_str()));
    let _ = writeln!(o, "  \"totalCalls\": {},", summary.total.calls);
    let _ = writeln!(o, "  \"totalCostUSD\": {},", float(costs.total));
    // Not in the reference (#5): a partial total is flagged, and the table that priced it named.
    let heur = &summary.footnote.heuristics;
    let _ = writeln!(
        o,
        "  \"totalCostIsFloor\": {},",
        costs.is_floor() || heur.any_ambiguous()
    );
    let unpriced: Vec<_> = costs.by_model.iter().filter(|(_, c)| c.is_none()).collect();
    if unpriced.is_empty() {
        o.push_str("  \"unpricedModels\": [],\n");
    } else {
        o.push_str("  \"unpricedModels\": [\n");
        let last = unpriced.len() - 1;
        for (i, (m, _)) in unpriced.iter().enumerate() {
            let comma = if i == last { "" } else { "," };
            let _ = writeln!(o, "    {}{comma}", string(m));
        }
        o.push_str("  ],\n");
    }
    price_table(costs, &mut o);

    heuristics(heur, &mut o);
    work_branch(summary.work_branch.as_ref(), &mut o);
    subagents(&summary.subagents, costs, &mut o);

    if sessions.is_empty() {
        o.push_str("  \"sessions\": [],\n");
    } else {
        o.push_str("  \"sessions\": [\n");
        for (i, (sid, s)) in sessions.iter().enumerate() {
            o.push_str("    {\n");
            let _ = writeln!(o, "      \"id\": {},", string(sid.as_str()));
            let _ = writeln!(o, "      \"origin\": {},", string(&s.origin));
            let _ = writeln!(o, "      \"first\": {},", string(&s.first));
            let _ = writeln!(o, "      \"last\": {},", string(&s.last));
            let _ = writeln!(o, "      \"calls\": {},", s.calls);
            let _ = writeln!(o, "      \"tokens\": {},", s.tokens);
            if s.agents.is_empty() {
                o.push_str("      \"agents\": []\n");
            } else {
                o.push_str("      \"agents\": [\n");
                let last = s.agents.len() - 1;
                for (j, a) in s.agents.iter().enumerate() {
                    let comma = if j == last { "" } else { "," };
                    let _ = writeln!(o, "        {}{comma}", string(a));
                }
                o.push_str("      ]\n");
            }
            o.push_str(if i + 1 == sessions.len() {
                "    }\n"
            } else {
                "    },\n"
            });
        }
        o.push_str("  ],\n");
    }

    if models.is_empty() {
        o.push_str("  \"models\": {}\n");
    } else {
        o.push_str("  \"models\": {\n");
        for (i, (model, b)) in models.iter().enumerate() {
            let _ = writeln!(o, "    {}: {{", string(model));
            let _ = writeln!(o, "      \"calls\": {},", b.calls);
            let _ = writeln!(o, "      \"input\": {},", b.input);
            let _ = writeln!(o, "      \"cacheWrite5m\": {},", b.cache_write_5m);
            let _ = writeln!(o, "      \"cacheWrite1h\": {},", b.cache_write_1h);
            let _ = writeln!(o, "      \"cacheRead\": {},", b.cache_read);
            let _ = writeln!(o, "      \"output\": {},", b.output);
            let _ = writeln!(o, "      \"thinking\": {},", b.thinking);
            let cost = costs.cost_for(model).map_or("null".to_owned(), float);
            let _ = writeln!(o, "      \"costUSD\": {cost}");
            o.push_str(if i + 1 == models.len() {
                "    }\n"
            } else {
                "    },\n"
            });
        }
        o.push_str("  }\n");
    }
    o.push('}');
    o
}

#[cfg(test)]
mod tests {
    use super::{float, repr};

    #[test]
    fn repr_matches_python_float_repr() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (100.0, "100.0"),
            (0.5, "0.5"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (123.456, "123.456"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.25e20, "1.25e+20"),
            (1e100, "1e+100"),
            (-2.5, "-2.5"),
        ];
        for (x, want) in cases {
            assert_eq!(repr(x), want, "{x:e}");
        }
    }

    #[test]
    fn float_rounds_to_four_places_with_ties_to_even_on_the_binary_value() {
        // 0.00005 is above the tie in binary; 0.5 * 2^-k values are exact ties.
        assert_eq!(float(0.00005), "0.0001");
        assert_eq!(float(0.00015), "0.0001"); // 0.000149999999999999995...
        assert_eq!(float(0.03125), "0.0312"); // exact tie 0.03125 rounds to even
        assert_eq!(float(0.09375), "0.0938"); // exact tie 0.09375 rounds to even
        assert_eq!(float(f64::NAN), "NaN");
        assert_eq!(float(f64::NEG_INFINITY), "-Infinity");
    }
}
