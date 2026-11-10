//! The markdown report, its marker wrapping and the PR-body splice.
//!
//! Number formatting follows the reference's f-strings. `{:.2}` and `{:.1}` in Rust round the
//! exact binary value, ties to even, as Python's `format` does (0.125 gives `0.12`, 2.675 gives
//! `2.67` in both); the tests pin the two cases.

use std::cmp::Reverse;
use std::fmt::Write as _;

use crate::model::{
    Costs, HeuristicBreakdown, Mode, PriceTableSource, RowCost, Summary, Workspace,
};

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
fn total_cost_cell(costs: &Costs, ambiguous: bool) -> String {
    let floor = if costs.is_floor() || ambiguous {
        "at least "
    } else {
        ""
    };
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

/// The footnote segment naming each heuristic that assigned at least one call, with its calls and
/// tokens. Zero entries are omitted, and so is the whole segment when none was used.
fn heuristic_note(h: &HeuristicBreakdown) -> String {
    let parts: Vec<String> = [
        ("session claim", h.session_claim),
        ("worktree recovery", h.worktree_recovery),
        ("origin recovery", h.origin_recovery),
        ("dispatcher roll-up", h.dispatcher_roll_up),
        ("timeline forward", h.timeline_forward),
        ("timeline carried", h.timeline_carried),
    ]
    .into_iter()
    .filter(|(_, c)| c.calls > 0)
    .map(|(name, c)| format!("{name} {} call(s) / {} tokens", n(c.calls), n(c.tokens)))
    .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" \u{b7} attributed by heuristic: {}", parts.join("; "))
    }
}

/// When some calls could not be attributed with certainty, the token total is only a floor
/// (Governance II): say so, and how much was ambiguous. Pushes nothing otherwise.
fn floor_statement(h: &HeuristicBreakdown, total_tokens: u64, o: &mut Vec<String>) {
    if !h.any_ambiguous() {
        return;
    }
    o.push(format!(
        "**Token total is at least {}**: {} token(s) in {} call(s) could not be attributed to a \
         branch with certainty.",
        n(total_tokens),
        n(h.ambiguous.tokens),
        n(h.ambiguous.calls),
    ));
    o.push(String::new());
}

/// A table cell from recorded text: a pipe would end the cell early.
fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

/// A subagent cost as text: a dash when there is no price (or no entry), "at least $X" for a
/// floor, else "$X". Money is `{:.2}` like the per-model cell.
fn row_cost_text(cost: Option<&RowCost>) -> String {
    match cost {
        Some(c) if c.is_unpriced() => "\u{2014}".to_owned(),
        Some(c) if c.is_floor() => format!("at least ${:.2}", c.known),
        Some(c) => format!("${:.2}", c.known),
        None => "\u{2014}".to_owned(),
    }
}

/// The model cell shared by the per-model and subagent tables: the id in backticks. An empty id
/// (a subagent row serialized before rows carried a model) is a dash, not a made-up label.
fn model_text(model: &str) -> String {
    if model.is_empty() {
        "\u{2014}".to_owned()
    } else {
        format!("`{model}`")
    }
}

/// The dispatched-subagent breakdown (#6), pushed as lines ending in a blank line.
///
/// Not in the reference, so it is omitted entirely when [`SubagentSummary::is_empty`]: that keeps
/// strict mode (which never fills it) and reports with no subagents byte-identical to before.
/// All totals come from the summary's accessors and count rolled-up records only. The
/// unattributed count spans every attributed record, not only this branch's, and the line says so
/// rather than imply otherwise, then says how many of those are inside this report's total.
fn subagent_section(summary: &Summary, costs: &Costs, show_cost: bool, o: &mut Vec<String>) {
    let sub = &summary.subagents;
    if sub.is_empty() {
        return;
    }
    o.push(format!(
        "### Subagent usage (rolled up to `{}`)",
        summary.branch
    ));
    o.push(String::new());
    let mut text = String::new();
    if !sub.rows.is_empty() {
        let (cost_head, cost_rule) = if show_cost {
            (" Est. cost |", "--:|")
        } else {
            ("", "")
        };
        o.push(format!(
            "| Workspace | Subagent | Model | Calls | Input | Cache write | Cache read | Output | \
             (thinking) | Tokens |{cost_head}"
        ));
        o.push(format!(
            "|---|---|---|--:|--:|--:|--:|--:|--:|--:|{cost_rule}"
        ));
        for (i, r) in sub.rows.iter().enumerate() {
            let ws = match &r.workspace {
                Workspace::Main => "main checkout".to_owned(),
                Workspace::Worktree(name) => format!("worktree `{}`", cell(name)),
                Workspace::Unknown => "unknown workspace".to_owned(),
            };
            // A row without a per-model split (older data) has no usage breakdown to show: a dash,
            // never a zero that would read as "none used".
            let usage = if r.by_model.is_empty() {
                vec!["\u{2014}".to_owned(); 5]
            } else {
                let t = r.total();
                vec![
                    n(t.input),
                    n(t.cache_write_5m + t.cache_write_1h),
                    n(t.cache_read),
                    n(t.output),
                    n(t.thinking),
                ]
            };
            let cost = if show_cost {
                format!(" {} |", row_cost_text(costs.subagent_row(i)))
            } else {
                String::new()
            };
            o.push(format!(
                "| {ws} | {} | {} | {} | {} | {} |{cost}",
                cell(&r.subagent),
                model_text(&r.model),
                n(r.calls),
                usage.join(" | "),
                n(r.tokens),
            ));
        }
        o.push(String::new());
        let est = if show_cost {
            format!("; est. cost {}", row_cost_text(Some(&costs.subagent_total)))
        } else {
            String::new()
        };
        let _ = writeln!(
            text,
            "{} tokens ({} calls) from {} dispatched subagent(s) across {} workspace(s) \
             ({} worktree(s)){est}.",
            n(sub.tokens()),
            n(sub.calls()),
            n(sub.dispatched()),
            n(sub.workspaces()),
            n(sub.worktrees()),
        );
    }
    if sub.unattributed_calls == 0 {
        text.push_str("0 subagent call(s) had no dispatch link.\n");
    } else {
        let in_total = if sub.unlinked_in_total_calls == 0 {
            "none of them are included in this report's total".to_owned()
        } else {
            format!(
                "{} call(s) ({} tokens) of them are included in this report's total via \
                 session claim",
                n(sub.unlinked_in_total_calls),
                n(sub.unlinked_in_total_tokens),
            )
        };
        let _ = writeln!(
            text,
            "{} subagent call(s) ({} tokens) across this repository's transcripts had no \
             dispatch link, so their branch is unverified (raw stamp kept); {in_total}.",
            n(sub.unattributed_calls),
            n(sub.unattributed_tokens),
        );
    }
    o.push(text);
}

/// The report (`render` in the reference): no trailing newline, no markers.
///
/// Tables are sorted here, stably, as the reference does: models by descending output, sessions
/// by `first`, attributions by descending tokens. Totals and costs are read, never recomputed.
#[must_use]
pub fn markdown(summary: &Summary, costs: &Costs, show_cost: bool) -> String {
    let total = &summary.total;
    let heur = &summary.footnote.heuristics;
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
            "| {} | {} | {} | {} | {} | {} | {} |{cell}",
            model_text(model),
            n(b.calls),
            n(b.input),
            n(b.cache_write_5m + b.cache_write_1h),
            n(b.cache_read),
            n(b.output),
            n(b.thinking),
        ));
    }
    let total_cell = if show_cost {
        total_cost_cell(costs, summary.footnote.heuristics.any_ambiguous())
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

    floor_statement(heur, total.tokens(), &mut o);

    let mode = if summary.mode == Mode::Timeline {
        " \u{b7} timeline mode"
    } else {
        ""
    };
    let mut note = format!(
        "<sub>Branch `{}`{mode} \u{b7} {} session(s), {} in a worktree \u{b7} {} of {} calls \
         from subagents{}",
        summary.branch,
        ordered.len(),
        summary.footnote.worktree_sessions,
        n(summary.footnote.subagent_calls),
        n(total.calls),
        heuristic_note(heur),
    );
    if show_cost {
        note.push_str(&price_note(costs));
        note.push_str(" \u{b7} est. cost at list API rates, not amounts billed</sub>");
    } else {
        note.push_str("</sub>");
    }
    o.push(note);
    o.push(String::new());

    subagent_section(summary, costs, show_cost, &mut o);

    attribution_details(summary, &mut o);
    o.join("\n")
}

/// The collapsed attribution table, sorted by descending tokens (stably).
fn attribution_details(summary: &Summary, o: &mut Vec<String>) {
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
