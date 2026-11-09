//! The markdown report, its marker wrapping and the PR-body splice.
//!
//! Number formatting follows the reference's f-strings. `{:.2}` and `{:.1}` in Rust round the
//! exact binary value, ties to even, as Python's `format` does (0.125 gives `0.12`, 2.675 gives
//! `2.67` in both); the tests pin the two cases.

use std::cmp::Reverse;
use std::fmt::Write as _;

use crate::model::{Costs, PriceTableSource, Summary};

/// Opens the region of a PR body this tool owns.
pub const START: &str = "<!-- token-report:start -->";
/// Closes the region of a PR body this tool owns.
pub const END: &str = "<!-- token-report:end -->";
/// The heading appended when a body has no markers yet.
pub const HEADING: &str = "## Token usage";

/// Python's `f"{x:,}"` for a non-negative integer.
fn n(x: u64) -> String {
    let digits = x.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Python's `ts[5:16].replace('T', ' ')`: a slice by characters, empty when `ts` is short.
fn clock(ts: &str) -> String {
    ts.chars()
        .skip(5)
        .take(11)
        .collect::<String>()
        .replace('T', " ")
}

/// Millions of tokens to one decimal, as `f"{x / 1e6:.1f}M"`. The cast is exact below 2^53.
#[expect(
    clippy::cast_precision_loss,
    reason = "token counts are far below 2^53, where u64 to f64 is exact"
)]
fn millions(tokens: u64) -> String {
    format!("{:.1}M", tokens as f64 / 1e6)
}

/// The total cost cell. A partial total is a floor (Governance II), so the cell says so itself.
fn total_cost_cell(costs: &Costs) -> String {
    let floor = if costs.is_floor() { "at least " } else { "" };
    format!(" **{floor}${:.2}** |", costs.total)
}

/// The footnote segments that say which prices produced the costs and what could not be priced.
/// An empty date means no table was consulted: say so rather than invent one.
fn price_note(costs: &Costs) -> String {
    let table = &costs.price_table;
    let mut note = if table.as_of.is_empty() {
        " \u{b7} price table date unknown".to_owned()
    } else {
        let source = match table.source {
            PriceTableSource::Bundled => "bundled table",
            PriceTableSource::Cache => "cached from LiteLLM",
            PriceTableSource::Override => ".tally/prices.toml",
        };
        format!(" \u{b7} prices as of {} ({source})", table.as_of)
    };
    if costs.is_floor() {
        let _ = write!(
            note,
            " \u{b7} {} model(s) unpriced, total is a floor",
            costs.unpriced_models
        );
    }
    note
}

/// The report (`render` in the reference): no trailing newline, no markers.
///
/// Tables are sorted here, stably, as the reference does: models by descending output, sessions
/// by `first`, attributions by descending tokens. Totals and costs are read, never recomputed.
#[must_use]
pub fn markdown(summary: &Summary, costs: &Costs, show_cost: bool) -> String {
    let total = &summary.total;
    let mut rows: Vec<_> = summary.by_model.iter().collect();
    rows.sort_by_key(|(_, b)| Reverse(b.output));
    let mut ordered: Vec<_> = summary.sessions.iter().collect();
    ordered.sort_by(|a, b| a.1.first.cmp(&b.1.first));

    let (cost_head, cost_rule) = if show_cost {
        (" Est. cost |", "--:|")
    } else {
        ("", "")
    };
    let mut o: Vec<String> = Vec::new();
    o.push(format!(
        "| Model | Calls | Input | Cache write | Cache read | Output | (thinking) |{cost_head}"
    ));
    o.push(format!("|---|--:|--:|--:|--:|--:|--:|{cost_rule}"));
    for (model, b) in rows {
        let cell = if show_cost {
            match costs.cost_for(model) {
                Some(c) => format!(" ${c:.2} |"),
                None => " \u{2014} |".to_owned(),
            }
        } else {
            String::new()
        };
        o.push(format!(
            "| `{model}` | {} | {} | {} | {} | {} | {} |{cell}",
            n(b.calls),
            n(b.input),
            n(b.cache_write_5m + b.cache_write_1h),
            n(b.cache_read),
            n(b.output),
            n(b.thinking),
        ));
    }
    let total_cell = if show_cost {
        total_cost_cell(costs)
    } else {
        String::new()
    };
    o.push(format!(
        "| **Total** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** |{total_cell}",
        n(total.calls),
        n(total.input),
        n(total.cache_write_5m + total.cache_write_1h),
        n(total.cache_read),
        n(total.output),
        n(total.thinking),
    ));
    o.push(String::new());

    o.push("| Session | Where | Agents | Span (UTC) | Calls | Tokens |".to_owned());
    o.push("|---|---|---|---|--:|--:|".to_owned());
    for (sid, s) in &ordered {
        let where_ = if s.origin == "main" {
            "main checkout"
        } else {
            "worktree"
        };
        let short: String = sid.as_str().chars().take(8).collect();
        o.push(format!(
            "| `{short}` | {where_} | {} | {} \u{2192} {} | {} | {} |",
            s.who(),
            clock(&s.first),
            clock(&s.last),
            n(s.calls),
            millions(s.tokens),
        ));
    }
    o.push(String::new());

    let mut note = format!(
        "<sub>Branch `{}` \u{b7} {} session(s), {} in a worktree \u{b7} {} of {} calls from \
         subagents \u{b7} {} tokens recovered from pre-branch/worktree checkouts",
        summary.branch,
        ordered.len(),
        summary.footnote.worktree_sessions,
        n(summary.footnote.subagent_calls),
        n(total.calls),
        millions(summary.footnote.off_branch_tokens),
    );
    if show_cost {
        note.push_str(&price_note(costs));
        note.push_str(" \u{b7} est. cost at list API rates, not amounts billed</sub>");
    } else {
        note.push_str("</sub>");
    }
    o.push(note);
    o.push(String::new());

    o.push("<details><summary>Breakdown by skill / agent</summary>".to_owned());
    o.push(String::new());
    o.push("| Attribution | Calls | Tokens |".to_owned());
    o.push("|---|--:|--:|".to_owned());
    let mut attrs: Vec<_> = summary.by_attribution.iter().collect();
    attrs.sort_by_key(|(_, b)| Reverse(b.tokens()));
    for (attr, b) in attrs {
        let mut line = String::new();
        // Writing to a String cannot fail.
        let _ = write!(line, "| {attr} | {} | {} |", n(b.calls), n(b.tokens()));
        o.push(line);
    }
    o.push(String::new());
    o.push("</details>".to_owned());
    o.join("\n")
}

/// Puts the markers on their own lines around a report (`wrap`). Markers are owned here, not by
/// [`markdown`], so a report can never be spliced unmarked.
#[must_use]
pub fn wrap(report: &str) -> String {
    format!("{START}\n{report}\n{END}")
}

/// Replaces the marked region of a PR body, or appends the section under [`HEADING`] (`splice`).
/// CRLF in the body becomes LF. Markers out of order count as absent.
#[must_use]
pub fn splice(body: &str, report: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let marked = wrap(report);
    if let (Some(i), Some(j)) = (body.find(START), body.find(END))
        && j > i
    {
        return format!("{}{marked}{}", &body[..i], &body[j + END.len()..]);
    }
    format!("{}\n\n{HEADING}\n\n{marked}\n", body.trim_end())
}
