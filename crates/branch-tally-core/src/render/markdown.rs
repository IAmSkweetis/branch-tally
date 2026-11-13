//! The markdown report, its marker wrapping and the PR-body splice.
//!
//! Number formatting follows the reference's f-strings. `{:.2}` and `{:.1}` in Rust round the
//! exact binary value, ties to even, as Python's `format` does (0.125 gives `0.12`, 2.675 gives
//! `2.67` in both); the tests pin the two cases.

use std::cmp::Reverse;
use std::fmt::Write as _;

use crate::model::{
    Costs, DefaultBranchSource, HeuristicBreakdown, Mode, PriceTableSource, RowCost, SessionId,
    SessionRow, Summary, WorkBranchRule, Workspace,
};

/// Opens the region of a PR body this tool owns.
pub const START: &str = "<!-- token-report:start -->";
/// Closes the region of a PR body this tool owns.
pub const END: &str = "<!-- token-report:end -->";
/// The heading appended when a body has no markers yet.
pub const HEADING: &str = "## Token usage";

/// Python's `f"{x:,}"` for a non-negative integer.
pub(super) fn n(x: u64) -> String {
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
        .filter(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '.' | '+'))
        .skip(5)
        .take(11)
        .collect::<String>()
        .replace('T', " ")
}

/// A session span end. A redacted summary already holds dates (or empty), not timestamps: print
/// them whole rather than slice them as clock times.
fn when(ts: &str, redacted: bool) -> String {
    if redacted {
        ts.chars()
            .filter(|c| c.is_ascii_digit() || *c == '-')
            .collect()
    } else {
        clock(ts)
    }
}

/// Millions of tokens to one decimal, as `f"{x / 1e6:.1f}M"`. The cast is exact below 2^53.
#[expect(
    clippy::cast_precision_loss,
    reason = "token counts are far below 2^53, where u64 to f64 is exact"
)]
pub(super) fn millions(tokens: u64) -> String {
    format!("{:.1}M", tokens as f64 / 1e6)
}

/// The total cost cell. A partial total is a floor (Governance II), so the cell says so itself.
fn total_cost_cell(costs: &Costs, ambiguous: bool, overflowed: bool) -> String {
    let floor = if costs.is_floor() || ambiguous || overflowed {
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
        format!(
            " \u{b7} prices as of {} ({source})",
            crate::text::replace_control_chars(&table.as_of)
        )
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
        ("captured hook data", h.captured),
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

/// The footnote segment stating which branches timeline mode treated as work branches, whenever
/// that is not the ordinary case (Governance II: a fallback or a custom rule is a heuristic the
/// report must declare). Empty for the default rule resolved from `origin/HEAD`, and outside
/// timeline mode.
fn work_branch_note(rule: Option<&WorkBranchRule>) -> String {
    match rule {
        None
        | Some(WorkBranchRule::ExcludesDefault {
            source: DefaultBranchSource::OriginHead,
            ..
        }) => String::new(),
        Some(WorkBranchRule::ExcludesDefault { branch, source }) => {
            let why = match source {
                DefaultBranchSource::InitDefaultBranch => {
                    "from init.defaultBranch; origin/HEAD was not available"
                }
                _ => "assumed; neither origin/HEAD nor init.defaultBranch was available",
            };
            format!(
                " \u{b7} work branches: all except default branch {} ({why})",
                code(branch, false)
            )
        }
        Some(WorkBranchRule::Custom { pattern }) => {
            format!(
                " \u{b7} work branches: custom pattern {}",
                code(pattern, false)
            )
        }
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

/// When a token or call total saturated, every total is a floor and unreliable (#96, Governance
/// II): say so. Pushes nothing otherwise.
fn overflow_statement(overflowed: bool, o: &mut Vec<String>) {
    if !overflowed {
        return;
    }
    o.push(
        "**Totals overflowed and are unreliable**: a token or call count exceeded the largest \
         value the tool can hold, so every total here is at least what is shown and the real \
         figure is unknown."
            .to_owned(),
    );
    o.push(String::new());
}

/// Recorded text on one line: a line break would end a table row, a heading or the footnote.
/// Every other control character (ESC, C1, bidi overrides) becomes U+FFFD, so none reaches a
/// terminal or reorders what a reader sees (#95).
fn one_line(s: &str) -> String {
    let flat = s.replace("\r\n", " ").replace(['\r', '\n'], " ");
    crate::text::replace_control_chars(&flat).into_owned()
}

/// Escapes each pipe for a GFM table cell (#39). The cell splitter treats a backslash as escaping
/// the next character, so a run of backslashes right before a pipe is padded to an even length
/// first. Otherwise the run would swallow the pipe's escape and the pipe would end the cell. In a
/// code span that shows one extra backslash, which is better than a broken table.
fn escape_pipes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '|' {
            let run = out.chars().rev().take_while(|&c| c == '\\').count();
            if run % 2 == 1 {
                out.push('\\');
            }
            out.push_str("\\|");
        } else {
            out.push(ch);
        }
    }
    out
}

/// Recorded text as a code span (#39). The fence is one backtick longer than the longest run of
/// backticks inside, padded with a space when the text starts or ends with a backtick or a space
/// (the parser strips one space from each side). `<` and `>` are literal inside a code span, so no
/// HTML gets through. `in_table` also escapes pipes, which GFM unescapes even inside code spans;
/// outside a table a pipe is harmless and a backslash would show. Empty text stays an empty pair
/// of backticks, as the reference prints it.
pub(super) fn code(s: &str, in_table: bool) -> String {
    let line = one_line(s);
    let body = if in_table { escape_pipes(&line) } else { line };
    let mut longest = 0;
    let mut run = 0;
    for ch in body.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let edge = |c: char| c == '`' || c == ' ';
    let pad = if body.starts_with(edge) || body.ends_with(edge) {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{body}{pad}{fence}")
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
        code(model, true)
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
        "### Subagent usage (rolled up to {})",
        code(&summary.branch, false)
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
                Workspace::Worktree(name) => format!("worktree {}", code(name, true)),
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
                    n(t.cache_write_5m.saturating_add(t.cache_write_1h)),
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
                code(&r.subagent, true),
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

/// The "who" cell: each agent as its own code span, or the em dash [`SessionRow::who`] uses when
/// there are none.
fn who_cell(s: &SessionRow) -> String {
    if s.agents.is_empty() {
        return "\u{2014}".to_owned();
    }
    s.agents
        .iter()
        .map(|a| code(a, true))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The sessions table, in the order given, ending in a blank line. A redacted summary's header
/// says the span is dates.
fn session_table(ordered: &[&(SessionId, SessionRow)], redacted: bool, o: &mut Vec<String>) {
    let span_head = if redacted {
        "Span (UTC date)"
    } else {
        "Span (UTC)"
    };
    o.push(format!(
        "| Session | Where | Agents | {span_head} | Calls | Tokens |"
    ));
    o.push("|---|---|---|---|--:|--:|".to_owned());
    for (sid, s) in ordered {
        let where_ = if s.origin == "main" {
            "main checkout"
        } else {
            "worktree"
        };
        let short: String = sid.as_str().chars().take(8).collect();
        o.push(format!(
            "| {} | {where_} | {} | {} \u{2192} {} | {} | {} |",
            code(&short, true),
            who_cell(s),
            when(&s.first, redacted),
            when(&s.last, redacted),
            n(s.calls),
            millions(s.tokens),
        ));
    }
    o.push(String::new());
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
            n(b.cache_write_5m.saturating_add(b.cache_write_1h)),
            n(b.cache_read),
            n(b.output),
            n(b.thinking),
        ));
    }
    let total_cell = if show_cost {
        total_cost_cell(
            costs,
            summary.footnote.heuristics.any_ambiguous(),
            summary.footnote.totals_overflowed,
        )
    } else {
        String::new()
    };
    o.push(format!(
        "| **Total** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** |{total_cell}",
        n(total.calls),
        n(total.input),
        n(total.cache_write_5m.saturating_add(total.cache_write_1h)),
        n(total.cache_read),
        n(total.output),
        n(total.thinking),
    ));
    o.push(String::new());

    session_table(&ordered, summary.redacted, &mut o);

    floor_statement(heur, total.tokens(), &mut o);
    overflow_statement(summary.footnote.totals_overflowed, &mut o);

    let mode = if summary.mode == Mode::Timeline {
        " \u{b7} timeline mode"
    } else {
        ""
    };
    let mut note = format!(
        "<sub>Branch {}{mode} \u{b7} {} session(s), {} in a worktree \u{b7} {} of {} calls \
         from subagents{}{}",
        code(&summary.branch, false),
        ordered.len(),
        summary.footnote.worktree_sessions,
        n(summary.footnote.subagent_calls),
        n(total.calls),
        heuristic_note(heur),
        work_branch_note(summary.work_branch.as_ref()),
    );
    if summary.footnote.totals_overflowed {
        note.push_str(" \u{b7} totals overflowed, unreliable");
    }
    if summary.redacted {
        note.push_str(" \u{b7} identifiers redacted");
    }
    if show_cost {
        note.push_str(&price_note(costs));
        note.push_str(" \u{b7} est. cost at list API rates, not amounts billed</sub>");
    } else {
        note.push_str("</sub>");
    }
    o.push(note);
    o.push(String::new());

    // #60: usage no branch could claim. Repository-wide, so it is not part of this branch's total.
    if summary.unstamped.calls > 0 {
        o.push(format!(
            "<sub>{} call(s) / {} tokens in this repository carry no branch stamp and are not \
             attributed to any branch.</sub>",
            n(summary.unstamped.calls),
            n(summary.unstamped.tokens),
        ));
        o.push(String::new());
    }

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
        let _ = write!(
            line,
            "| {} | {} | {} |",
            code(attr, true),
            n(b.calls),
            n(b.tokens())
        );
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
/// CRLF in the body becomes LF. A marker counts only on a line of its own (trailing whitespace
/// allowed), as [`wrap`] writes it: rendered values are flattened to one line, so a name that
/// spells a marker can't end the region, and prose quoting a marker is left alone. The region is
/// the first START line through the first END line after it. Markers out of order count as absent.
#[must_use]
pub fn splice(body: &str, report: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let marked = wrap(report);
    if let Some((i, _)) = marker_line(&body, 0, START)
        && let Some((_, j)) = marker_line(&body, i, END)
    {
        return format!("{}{marked}{}", &body[..i], &body[j..]);
    }
    format!("{}\n\n{HEADING}\n\n{marked}\n", body.trim_end())
}

/// The first line at or after byte `from` that is exactly `marker` once trailing whitespace is
/// trimmed, as the byte range of the line without its newline.
fn marker_line(body: &str, from: usize, marker: &str) -> Option<(usize, usize)> {
    let mut start = from;
    for line in body[from..].split('\n') {
        let end = start + line.len();
        if line.trim_end() == marker {
            return Some((start, end));
        }
        start = end + 1;
    }
    None
}
