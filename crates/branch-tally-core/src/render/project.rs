//! The project report as Markdown (with a Mermaid chart), JSON and a standalone SVG.
//!
//! Pure formatting of a [`ProjectReport`]: every number is read from it, none is summed or
//! priced here. Only branch names, dates and numbers are printed (Governance III), and every
//! branch name is escaped for the format it lands in.

use std::fmt::Write as _;

use super::json::{float, string};
use super::markdown::{code, millions, n};
use crate::model::{BranchRow, HeuristicBreakdown, HeuristicCount, ProjectReport, RowCost};

/// The attribution heuristics in report order, with the Markdown name and the JSON key.
fn heuristic_entries(h: &HeuristicBreakdown) -> [(&'static str, &'static str, HeuristicCount); 8] {
    [
        ("session claim", "sessionClaim", h.session_claim),
        ("worktree recovery", "worktreeRecovery", h.worktree_recovery),
        ("captured hook data", "captured", h.captured),
        ("origin recovery", "originRecovery", h.origin_recovery),
        (
            "dispatcher roll-up",
            "dispatcherRollUp",
            h.dispatcher_roll_up,
        ),
        ("timeline forward", "timelineForward", h.timeline_forward),
        ("timeline carried", "timelineCarried", h.timeline_carried),
        ("ambiguous", "ambiguous", h.ambiguous),
    ]
}

/// A timestamp truncated to its day. Anything outside the characters a date or time uses is
/// dropped, so a malformed value cannot reach Mermaid or XML as markup.
fn day(ts: &str) -> String {
    ts.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':'))
        .take(10)
        .collect()
}

/// The value the chart plots (its last point) and whether it must read "at least": the data has
/// a floor, or undated branches mean the dated-only line under-counts the project.
fn plotted(report: &ProjectReport) -> Option<(f64, bool)> {
    let last = report.series.last()?;
    Some((
        last.cumulative_cost,
        report.is_floor() || report.undated_branches > 0,
    ))
}

/// Unique, date-based x-axis labels. A day that occurs more than once gets an ordinal (`#1`,
/// `#2`, ...) on every occurrence, because Mermaid's band scale collapses duplicate categories.
fn unique_labels(report: &ProjectReport) -> Vec<String> {
    let days: Vec<String> = report.series.iter().map(|p| day(&p.date)).collect();
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    report
        .series
        .iter()
        .zip(&days)
        .map(|(p, d)| {
            let total = days.iter().filter(|x| *x == d).count();
            let k = seen.entry(d.as_str()).or_insert(0);
            *k += 1;
            let ord = if total > 1 {
                format!(" #{k}")
            } else {
                String::new()
            };
            format!("\"{d}{ord}{}\"", if p.is_floor { "*" } else { "" })
        })
        .collect()
}

/// A cost as text: "at least $X" for a floor, else "$X". Money is `{:.2}` like the summary.
fn cost_text(cost: &RowCost, floor: bool) -> String {
    if floor {
        format!("at least ${:.2}", cost.known)
    } else {
        format!("${:.2}", cost.known)
    }
}

/// Whether a row has nothing priced at all, so there is no number to show.
fn row_unpriced(row: &BranchRow) -> bool {
    row.cost.is_unpriced() && row.unpriced_models.is_empty()
}

fn row_cost_cell(row: &BranchRow) -> String {
    if row_unpriced(row) {
        "\u{2014}".to_owned()
    } else {
        cost_text(&row.cost, row.is_floor())
    }
}

/// The project report as Markdown: the total, a per-branch table, the notes and a Mermaid chart.
///
/// A floor anywhere labels the total "at least". Rows may sum to more than the total, because a
/// call shared by two branches is in both rows and once in the total; the overlap note says so.
#[must_use]
pub fn project_markdown(report: &ProjectReport) -> String {
    let t = &report.totals;
    let mut o = String::new();
    o.push_str("## Project cost\n\n");
    if report.rows.is_empty() {
        o.push_str("No branches recorded in the ledger.\n");
        return o;
    }
    let floor = if report.is_floor() { "at least " } else { "" };
    let _ = writeln!(
        o,
        "Project total: **{floor}${:.2}** \u{b7} {} call(s) \u{b7} {} tokens ({})\n",
        t.cost.known,
        n(t.usage.calls),
        n(t.usage.tokens()),
        millions(t.usage.tokens())
    );
    if report.is_overflowed() {
        o.push_str(
            "**Totals overflowed and are unreliable**: a token or call count exceeded the largest \
             value the tool can hold, so every total here is at least what is shown and the real \
             figure is unknown.\n\n",
        );
    }
    o.push_str(
        "| Branch | Last call | Calls | Tokens | Cost |\n| --- | --- | ---: | ---: | ---: |\n",
    );
    for r in &report.rows {
        let date = r
            .last_call
            .as_ref()
            .map_or("undated".to_owned(), |t| day(t.as_str()));
        let _ = writeln!(
            o,
            "| {} | {date} | {} | {} | {} |",
            code(&r.branch, true),
            n(r.usage.calls),
            n(r.usage.tokens()),
            row_cost_cell(r)
        );
    }
    o.push('\n');
    notes(report, &mut o);
    chart(report, &mut o);
    o
}

/// The notes under the table: overlap, unpriced models, uncertain calls and undated branches.
fn notes(report: &ProjectReport, o: &mut String) {
    let t = &report.totals;
    let before = o.len();
    if t.overlap.calls > 0 {
        let _ = writeln!(
            o,
            "- Shared calls: {} call(s) ({} tokens, {}) were counted more than once in the rows \
             because two or more branches claim them. The project total counts each once, so \
             the rows may sum to more than the total.",
            n(t.overlap.calls),
            n(t.overlap.tokens),
            cost_text(&t.overlap.cost, t.overlap.cost.is_floor())
        );
    }
    if !t.unpriced_models.is_empty() {
        let names: Vec<String> = t.unpriced_models.iter().map(|m| code(m, false)).collect();
        let _ = writeln!(
            o,
            "- {} model(s) unpriced, total is a floor: {}",
            t.unpriced_models.len(),
            names.join(", ")
        );
    }
    let heuristic: Vec<String> = heuristic_entries(&t.uncertain)
        .into_iter()
        .filter(|(_, key, c)| *key != "ambiguous" && c.calls > 0)
        .map(|(name, _, c)| format!("{name} {} call(s) / {} tokens", n(c.calls), n(c.tokens)))
        .collect();
    if !heuristic.is_empty() {
        let _ = writeln!(o, "- Attributed by heuristic: {}", heuristic.join("; "));
    }
    let amb = t.uncertain.ambiguous;
    if amb.calls > 0 {
        let _ = writeln!(
            o,
            "- Calls that could not be placed with certainty: {} call(s) / {} tokens. Token \
             totals are at least what is shown.",
            n(amb.calls),
            n(amb.tokens)
        );
    }
    if report.undated_branches > 0 {
        let _ = writeln!(
            o,
            "- {} branch(es) have no timestamp and are left off the chart, which covers dated \
             branches only and so shows at least the cost of those.",
            n(report.undated_branches)
        );
    }
    let early: Vec<String> = report
        .rows
        .iter()
        .filter(|r| r.last_call.is_some() && r.undated_calls > 0)
        .map(|r| {
            format!(
                "{} ({} call(s))",
                code(&r.branch, false),
                n(r.undated_calls)
            )
        })
        .collect();
    if !early.is_empty() {
        let _ = writeln!(
            o,
            "- {} call(s) have no usable timestamp. These branches may end later than their \
             last call shown, because those calls could not be dated: {}",
            n(t.undated_calls),
            early.join(", ")
        );
    }
    if o.len() > before {
        o.push('\n');
    }
}

/// The Mermaid `xychart-beta` of the cumulative series, one point per branch. Labels are dates:
/// a branch name never enters the chart source. A floor point has `*` on its label.
fn chart(report: &ProjectReport, o: &mut String) {
    if report.series.is_empty() {
        o.push_str("No dated branches, so there is no cost chart.\n");
        return;
    }
    let max = report
        .series
        .iter()
        .map(|p| p.cumulative_cost)
        .fold(0.0_f64, f64::max);
    let top = if max.is_finite() && max > 0.0 {
        max.ceil()
    } else {
        1.0
    };
    let any_floor = report.series.iter().any(|p| p.is_floor);
    let labels = unique_labels(report);
    let values: Vec<String> = report
        .series
        .iter()
        .map(|p| format!("{:.2}", p.cumulative_cost))
        .collect();
    let title = match plotted(report) {
        Some((v, true)) => format!("Cumulative cost (USD, at least ${v:.2})"),
        _ => "Cumulative cost (USD)".to_owned(),
    };
    o.push_str("```mermaid\nxychart-beta\n");
    let _ = writeln!(o, "    title \"{title}\"");
    let _ = writeln!(o, "    x-axis [{}]", labels.join(", "));
    let _ = writeln!(o, "    y-axis \"USD\" 0 --> {top:.0}");
    let _ = writeln!(o, "    line [{}]", values.join(", "));
    o.push_str("```\n");
    if any_floor {
        o.push_str(
            "\nPoints marked * are floors: that branch or an earlier one has an unpriced model, \
             so the real cost is at least this.\n",
        );
    }
}

fn heuristics_json(h: &HeuristicBreakdown) -> String {
    let mut parts: Vec<String> = heuristic_entries(h)
        .into_iter()
        .map(|(_, key, c)| {
            format!(
                "\"{key}\": {{ \"calls\": {}, \"tokens\": {} }}",
                c.calls, c.tokens
            )
        })
        .collect();
    parts.push(format!("\"isFloor\": {}", h.any_ambiguous()));
    format!("{{ {} }}", parts.join(", "))
}

fn strings_json(items: &[String]) -> String {
    let v: Vec<String> = items.iter().map(|s| string(s)).collect();
    format!("[{}]", v.join(", "))
}

/// The project report as JSON, formatted like [`super::json`]: two-space indent, insertion-order
/// keys, ASCII-escaped strings, floats rounded to four places.
///
/// `totalCostIsFloor` is true when any figure is a floor. A branch with no priced model has a
/// `null` `costUSD`; one with no timestamp has a `null` `lastCall`.
#[must_use]
pub fn project_json(report: &ProjectReport) -> String {
    let t = &report.totals;
    let mut o = String::from("{\n");
    let _ = writeln!(o, "  \"totalCalls\": {},", t.usage.calls);
    let _ = writeln!(o, "  \"totalTokens\": {},", t.usage.tokens());
    let _ = writeln!(
        o,
        "  \"totalTokensIsFloor\": {},",
        t.uncertain.any_ambiguous() || report.is_overflowed()
    );
    if report.is_overflowed() {
        o.push_str("  \"totalsOverflowed\": true,\n");
    }
    let _ = writeln!(o, "  \"totalCostUSD\": {},", float(t.cost.known));
    let _ = writeln!(o, "  \"totalCostIsFloor\": {},", report.is_floor());
    let _ = writeln!(
        o,
        "  \"unpricedModels\": {},",
        strings_json(&t.unpriced_models)
    );
    let _ = writeln!(
        o,
        "  \"overlap\": {{ \"calls\": {}, \"tokens\": {}, \"costUSD\": {}, \"costIsFloor\": {} }},",
        t.overlap.calls,
        t.overlap.tokens,
        float(t.overlap.cost.known),
        t.overlap.cost.is_floor()
    );
    let _ = writeln!(o, "  \"heuristics\": {},", heuristics_json(&t.uncertain));
    let _ = writeln!(o, "  \"undatedBranches\": {},", report.undated_branches);
    let _ = writeln!(o, "  \"undatedCalls\": {},", t.undated_calls);
    if report.rows.is_empty() {
        o.push_str("  \"branches\": [],\n");
    } else {
        o.push_str("  \"branches\": [\n");
        for (i, r) in report.rows.iter().enumerate() {
            let last = r
                .last_call
                .as_ref()
                .map_or("null".to_owned(), |t| string(t.as_str()));
            let usd = if row_unpriced(r) {
                "null".to_owned()
            } else {
                float(r.cost.known)
            };
            let comma = if i + 1 == report.rows.len() { "" } else { "," };
            let overflow = if r.totals_overflowed {
                ", \"totalsOverflowed\": true"
            } else {
                ""
            };
            let _ = writeln!(
                o,
                "    {{ \"branch\": {}, \"lastCall\": {last}, \"calls\": {}, \"tokens\": {}, \
                 \"costUSD\": {usd}, \"costIsFloor\": {}, \"unpricedModels\": {}, \
                 \"heuristics\": {}, \"undatedCalls\": {}{overflow} }}{comma}",
                string(&r.branch),
                r.usage.calls,
                r.usage.tokens(),
                r.is_floor(),
                strings_json(&r.unpriced_models),
                heuristics_json(&r.uncertain),
                r.undated_calls
            );
        }
        o.push_str("  ],\n");
    }
    if report.series.is_empty() {
        o.push_str("  \"series\": []\n");
    } else {
        o.push_str("  \"series\": [\n");
        for (i, p) in report.series.iter().enumerate() {
            let comma = if i + 1 == report.series.len() {
                ""
            } else {
                ","
            };
            let _ = writeln!(
                o,
                "    {{ \"date\": {}, \"branch\": {}, \"cumulativeCostUSD\": {}, \"isFloor\": {} }}{comma}",
                string(&p.date),
                string(&p.branch),
                float(p.cumulative_cost),
                p.is_floor
            );
        }
        o.push_str("  ]\n");
    }
    o.push_str("}\n");
    o
}

/// Text for an XML text node or attribute value. Control characters, which XML 1.0 forbids,
/// become spaces.
fn xml(s: &str) -> String {
    let s = crate::text::replace_control_chars(s);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 360.0;
const LEFT: f64 = 72.0;
const RIGHT: f64 = 24.0;
/// Baseline of the first footer line and the step between footer lines.
const FOOTER_FIRST_Y: f64 = HEIGHT - 28.0;
const FOOTER_STEP: f64 = 16.0;
/// Room kept below the last footer baseline for descenders.
const FOOTER_MARGIN: f64 = 12.0;

/// Canvas height for `n` footer lines: the base height, grown so the last line stays inside.
#[expect(
    clippy::cast_precision_loss,
    reason = "footer line counts are at most a handful, far below 2^53"
)]
fn footer_canvas_height(n: usize) -> f64 {
    let last = FOOTER_FIRST_Y + FOOTER_STEP * n.saturating_sub(1) as f64;
    HEIGHT.max(last + FOOTER_MARGIN)
}
const TOP: f64 = 56.0;
const BOTTOM: f64 = 72.0;

/// The cumulative cost chart as a standalone SVG for a README.
///
/// Deterministic text with no external resources. Colours come from a `<style>` block with a
/// dark-scheme override, so it reads on light and dark GitHub backgrounds. A floor point is a
/// hollow circle and the segment into it is dashed; the title says "at least" for any floor.
/// Every interpolated string is XML-escaped.
#[must_use]
pub fn project_svg(report: &ProjectReport) -> String {
    let heading = match plotted(report) {
        Some((v, floor)) => format!(
            "Cumulative cost: {}${v:.2}",
            if floor { "at least " } else { "" }
        ),
        None => "Cumulative cost".to_owned(),
    };
    let mut footer = Vec::new();
    if report.series.iter().any(|p| p.is_floor) {
        footer.push("hollow = floor (at least), dashed line leads to a floor".to_owned());
    }
    if report.is_overflowed() {
        footer.push("totals overflowed, unreliable".to_owned());
    }
    if report.undated_branches > 0 {
        footer.push(format!(
            "{} undated branch(es) not shown",
            report.undated_branches
        ));
    }
    if report.totals.undated_calls > 0 {
        footer.push(format!(
            "{} call(s) with no usable timestamp, branch dates may be early",
            report.totals.undated_calls
        ));
    }
    let height = footer_canvas_height(footer.len());
    let mut o = String::new();
    let _ = writeln!(
        o,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {WIDTH:.0} {height:.0}\" \
         width=\"{WIDTH:.0}\" height=\"{height:.0}\" role=\"img\">"
    );
    let _ = writeln!(o, "<title>{}</title>", xml(&heading));
    o.push_str(
        "<style>\n\
         .t{fill:#57606a;font:12px sans-serif}\n\
         .h{font-size:15px;font-weight:600}\n\
         .grid{stroke:#8c959f;stroke-opacity:.4}\n\
         .seg{stroke:#0969da;stroke-width:2;fill:none}\n\
         .dash{stroke-dasharray:6 4}\n\
         .dot{fill:#0969da}\n\
         .floor{fill:none;stroke:#0969da;stroke-width:2}\n\
         @media (prefers-color-scheme: dark){\n\
         .t{fill:#c9d1d9}\n\
         .seg{stroke:#58a6ff}\n\
         .dot{fill:#58a6ff}\n\
         .floor{stroke:#58a6ff}\n\
         }\n\
         </style>\n",
    );
    let _ = writeln!(
        o,
        "<text class=\"t h\" x=\"{LEFT:.1}\" y=\"28\">{}</text>",
        xml(&heading)
    );
    let plot_w = WIDTH - LEFT - RIGHT;
    let plot_h = HEIGHT - TOP - BOTTOM;
    if report.series.is_empty() {
        let _ = writeln!(
            o,
            "<text class=\"t\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">No dated branches</text>",
            LEFT + plot_w / 2.0,
            TOP + plot_h / 2.0
        );
    } else {
        series_svg(report, plot_w, plot_h, &mut o);
    }
    let mut y = FOOTER_FIRST_Y;
    for line in &footer {
        let _ = writeln!(
            o,
            "<text class=\"t\" x=\"{LEFT:.1}\" y=\"{y:.1}\">{}</text>",
            xml(line)
        );
        y += FOOTER_STEP;
    }
    o.push_str("</svg>\n");
    o
}

/// Position `i` of `count` evenly spaced along the plot width. The casts are exact for any
/// plausible branch count (far below 2^53).
#[expect(
    clippy::cast_precision_loss,
    reason = "branch counts are far below 2^53, where usize to f64 is exact"
)]
fn x_at(i: usize, count: usize, plot_w: f64) -> f64 {
    if count <= 1 {
        LEFT + plot_w / 2.0
    } else {
        LEFT + plot_w * i as f64 / (count - 1) as f64
    }
}

/// Gridlines, the line, the points and the axis labels for a non-empty series.
fn series_svg(report: &ProjectReport, plot_w: f64, plot_h: f64, o: &mut String) {
    let base = TOP + plot_h;
    let max = report
        .series
        .iter()
        .map(|p| p.cumulative_cost)
        .fold(0.0_f64, f64::max);
    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        1.0
    };
    for k in 0..=2_u32 {
        let frac = f64::from(k) / 2.0;
        let y = base - plot_h * frac;
        let _ = writeln!(
            o,
            "<line class=\"grid\" x1=\"{LEFT:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            LEFT + plot_w
        );
        let _ = writeln!(
            o,
            "<text class=\"t\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">${:.2}</text>",
            LEFT - 8.0,
            y + 4.0,
            max * frac
        );
    }
    let count = report.series.len();
    let coords: Vec<(f64, f64)> = report
        .series
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let y = base - plot_h * (p.cumulative_cost / max).clamp(0.0, 1.0);
            (x_at(i, count, plot_w), y)
        })
        .collect();
    for (i, pair) in coords.windows(2).enumerate() {
        let dash = if report.series[i + 1].is_floor {
            " dash"
        } else {
            ""
        };
        let _ = writeln!(
            o,
            "<line class=\"seg{dash}\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            pair[0].0, pair[0].1, pair[1].0, pair[1].1
        );
    }
    for (p, (x, y)) in report.series.iter().zip(&coords) {
        let class = if p.is_floor { "floor" } else { "dot" };
        let tip = format!(
            "{}, {}: {}${:.2}",
            p.branch,
            day(&p.date),
            if p.is_floor { "at least " } else { "" },
            p.cumulative_cost
        );
        let _ = writeln!(
            o,
            "<circle class=\"{class}\" cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\"><title>{}</title></circle>",
            xml(&tip)
        );
    }
    let anchor = if count == 1 { "middle" } else { "start" };
    let _ = writeln!(
        o,
        "<text class=\"t\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"{anchor}\">{}</text>",
        coords[0].0,
        base + 20.0,
        day(&report.series[0].date)
    );
    if count > 1 {
        let _ = writeln!(
            o,
            "<text class=\"t\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>",
            coords[count - 1].0,
            base + 20.0,
            day(&report.series[count - 1].date)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BranchRow, Bucket, HeuristicCount, ProjectTotals, RowCost, SeriesPoint, SharedOverlap,
    };

    fn cost(known: f64, unpriced: u64) -> RowCost {
        RowCost {
            known,
            priced_models: 1,
            unpriced_models: unpriced,
        }
    }

    fn usage(calls: u64, output: u64) -> Bucket {
        Bucket {
            output,
            calls,
            ..Bucket::default()
        }
    }

    fn row(branch: &str, last: Option<&str>, calls: u64, tokens: u64, usd: f64) -> BranchRow {
        BranchRow {
            branch: branch.to_owned(),
            last_call: last.and_then(|t| t.parse().ok()),
            usage: usage(calls, tokens),
            cost: cost(usd, 0),
            ..BranchRow::default()
        }
    }

    fn point(date: &str, branch: &str, cum: f64, floor: bool) -> SeriesPoint {
        SeriesPoint {
            date: date.to_owned(),
            branch: branch.to_owned(),
            cumulative_cost: cum,
            is_floor: floor,
        }
    }

    /// Two dated branches, exact prices, no overlap.
    fn exact() -> ProjectReport {
        ProjectReport {
            rows: vec![
                row("feat/a", Some("2026-01-02T03:04:05Z"), 2, 1_500, 1.25),
                row("feat/b", Some("2026-02-03T00:00:00Z"), 3, 2_500, 2.5),
            ],
            totals: ProjectTotals {
                usage: usage(5, 4_000),
                cost: cost(3.75, 0),
                ..ProjectTotals::default()
            },
            series: vec![
                point("2026-01-02T03:04:05Z", "feat/a", 1.25, false),
                point("2026-02-03T00:00:00Z", "feat/b", 3.75, false),
            ],
            undated_branches: 0,
        }
    }

    /// Floor, unpriced model, overlap, uncertain calls and an undated branch together.
    fn messy() -> ProjectReport {
        let mut r = exact();
        r.rows[1].unpriced_models = vec!["mystery-1".to_owned()];
        r.rows[1].cost = cost(2.5, 1);
        r.rows.push(row("scratch", None, 1, 10, 0.0));
        r.totals.cost = cost(3.75, 1);
        r.totals.unpriced_models = vec!["mystery-1".to_owned()];
        r.totals.uncertain.timeline_forward = HeuristicCount {
            calls: 2,
            tokens: 700,
        };
        r.totals.uncertain.ambiguous = HeuristicCount {
            calls: 1,
            tokens: 50,
        };
        r.totals.overlap = SharedOverlap {
            calls: 1,
            tokens: 100,
            cost: cost(0.5, 0),
        };
        r.series[1].is_floor = true;
        r.undated_branches = 1;
        r
    }

    const HOSTILE: &str = "x<script>&\"'|`\\]\n-->";

    fn hostile() -> ProjectReport {
        let mut r = exact();
        r.rows[0].branch = HOSTILE.to_owned();
        r.series[0].branch = HOSTILE.to_owned();
        r
    }

    // ---- Markdown ----

    #[test]
    fn markdown_exact_has_total_table_and_chart_without_floor_words() {
        let md = project_markdown(&exact());
        assert!(md.contains("**$3.75**"), "{md}");
        assert!(!md.contains("at least"), "{md}");
        assert!(
            md.contains("| Branch | Last call | Calls | Tokens | Cost |"),
            "{md}"
        );
        assert!(
            md.contains("| `feat/a` | 2026-01-02 | 2 | 1,500 | $1.25 |"),
            "{md}"
        );
        assert!(md.contains("```mermaid\nxychart-beta"), "{md}");
        assert!(
            md.contains("x-axis [\"2026-01-02\", \"2026-02-03\"]"),
            "{md}"
        );
        assert!(md.contains("line [1.25, 3.75]"), "{md}");
    }

    #[test]
    fn markdown_floor_unpriced_overlap_uncertain_and_undated_are_all_shown() {
        let md = project_markdown(&messy());
        assert!(md.contains("**at least $3.75**"), "{md}");
        assert!(md.contains("at least $2.50"), "{md}");
        assert!(md.contains("1 model(s) unpriced, total is a floor"), "{md}");
        assert!(md.contains("`mystery-1`"), "{md}");
        assert!(md.contains("counted more than once in the rows"), "{md}");
        assert!(
            md.contains("timeline forward 2 call(s) / 700 tokens"),
            "{md}"
        );
        assert!(
            md.contains("could not be placed with certainty: 1 call(s) / 50 tokens"),
            "{md}"
        );
        assert!(md.contains("1 branch(es) have no timestamp"), "{md}");
        assert!(md.contains("| `scratch` | undated |"), "{md}");
        assert!(
            md.contains("x-axis [\"2026-01-02\", \"2026-02-03*\"]"),
            "{md}"
        );
        assert!(md.contains("Points marked *"), "{md}");
    }

    #[test]
    fn markdown_floor_from_a_row_alone_labels_the_total_at_least() {
        let mut r = exact();
        r.rows[0].cost = cost(1.25, 1);
        assert!(project_markdown(&r).contains("**at least $3.75**"));
    }

    #[test]
    fn markdown_empty_ledger_says_so_and_draws_no_chart() {
        let md = project_markdown(&ProjectReport::default());
        assert!(md.contains("No branches recorded"), "{md}");
        assert!(!md.contains("mermaid"), "{md}");
    }

    #[test]
    fn markdown_hostile_branch_stays_inside_code_span_and_table() {
        let md = project_markdown(&hostile());
        let table_rows: Vec<&str> = md.lines().filter(|l| l.starts_with("| ")).collect();
        // Header, separator and the two branches: a newline in the name must not add a line.
        assert_eq!(table_rows.len(), 4, "{md}");
        assert!(table_rows[2].starts_with("| ``x<script>"), "{md}");
        assert!(table_rows[2].contains("\\|"), "{md}");
        assert!(!md.contains("\n-->"), "{md}");
        // The chart labels use dates, never the branch name.
        let chart = md.split("```mermaid").nth(1).unwrap_or("");
        assert!(!chart.contains("script"), "{md}");
    }

    #[test]
    fn control_and_bidi_characters_never_reach_project_markdown_or_svg() {
        let mut r = exact();
        let name = "b\u{1b}[31m\u{202e}x\u{85}".to_owned();
        r.rows[0].branch.clone_from(&name);
        r.series[0].branch = name;
        for out in [project_markdown(&r), project_svg(&r)] {
            let bad: Vec<char> = out
                .chars()
                .filter(|&c| c != '\n' && crate::text::is_unsafe_control(c))
                .collect();
            assert!(bad.is_empty(), "{bad:?}");
        }
        assert!(project_markdown(&r).contains("`b\u{FFFD}[31m\u{FFFD}x\u{FFFD}`"));
    }

    // ---- JSON ----

    #[test]
    fn json_exact_has_flags_and_series() {
        let j = project_json(&exact());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["totalCostUSD"], 3.75);
        assert_eq!(v["totalCostIsFloor"], false);
        assert_eq!(v["totalCalls"], 5);
        assert_eq!(v["undatedBranches"], 0);
        assert_eq!(v["branches"][0]["branch"], "feat/a");
        assert_eq!(v["branches"][0]["costUSD"], 1.25);
        assert_eq!(v["branches"][0]["costIsFloor"], false);
        assert_eq!(v["series"][1]["cumulativeCostUSD"], 3.75);
        assert!(j.contains("\n  \"totalCalls\": 5,"), "{j}");
    }

    #[test]
    fn json_floor_unpriced_overlap_uncertain_undated() {
        let j = project_json(&messy());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["totalCostIsFloor"], true);
        assert_eq!(v["unpricedModels"][0], "mystery-1");
        assert_eq!(v["branches"][1]["costIsFloor"], true);
        assert_eq!(v["branches"][2]["lastCall"], serde_json::Value::Null);
        assert_eq!(v["series"][1]["isFloor"], true);
        assert_eq!(v["series"][0]["isFloor"], false);
        assert_eq!(v["overlap"]["calls"], 1);
        assert_eq!(v["overlap"]["costUSD"], 0.5);
        assert_eq!(v["heuristics"]["timelineForward"]["calls"], 2);
        assert_eq!(v["heuristics"]["ambiguous"]["tokens"], 50);
        assert_eq!(v["heuristics"]["isFloor"], true);
        assert_eq!(v["undatedBranches"], 1);
    }

    #[test]
    fn json_empty_ledger_and_hostile_names_are_valid() {
        let v: serde_json::Value =
            serde_json::from_str(&project_json(&ProjectReport::default())).unwrap();
        assert_eq!(v["branches"], serde_json::json!([]));
        assert_eq!(v["series"], serde_json::json!([]));
        assert_eq!(v["totalCostIsFloor"], false);
        let v: serde_json::Value = serde_json::from_str(&project_json(&hostile())).unwrap();
        assert_eq!(v["branches"][0]["branch"], HOSTILE);
    }

    // ---- SVG ----

    #[test]
    fn svg_exact_is_deterministic_wellformed_and_labelled() {
        let a = project_svg(&exact());
        assert_eq!(a, project_svg(&exact()));
        assert!(a.starts_with("<svg "), "{a}");
        assert!(a.trim_end().ends_with("</svg>"), "{a}");
        assert!(a.contains("xmlns=\"http://www.w3.org/2000/svg\""), "{a}");
        assert!(a.contains("<title>Cumulative cost: $3.75</title>"), "{a}");
        assert!(a.contains("prefers-color-scheme: dark"), "{a}");
        assert!(a.contains("2026-01-02"), "{a}");
        assert!(!a.contains("at least"), "{a}");
        assert_eq!(a.matches("<circle").count(), 2, "{a}");
        assert!(!a.contains("class=\"floor\""), "{a}");
    }

    #[test]
    fn svg_floor_is_labelled_and_floor_points_are_distinct() {
        let s = project_svg(&messy());
        assert!(
            s.contains("<title>Cumulative cost: at least $3.75</title>"),
            "{s}"
        );
        assert_eq!(s.matches("<circle class=\"floor\"").count(), 1, "{s}");
        assert!(s.contains("hollow = floor"), "{s}");
        assert!(s.contains("1 undated branch(es) not shown"), "{s}");
    }

    #[test]
    fn svg_empty_ledger_has_a_message_and_no_points() {
        let s = project_svg(&ProjectReport::default());
        assert!(s.contains("No dated branches"), "{s}");
        assert!(!s.contains("<circle"), "{s}");
    }

    #[test]
    fn svg_hostile_branch_is_xml_escaped() {
        let mut r = hostile();
        r.series[0].is_floor = false;
        let s = project_svg(&r);
        assert!(!s.contains("<script"), "{s}");
        assert!(s.contains("x&lt;script&gt;&amp;&quot;&apos;|"), "{s}");
    }

    // ---- chart honesty (#87 fix round) ----

    fn mermaid_block(md: &str) -> String {
        md.split("```mermaid").nth(1).unwrap_or("").to_owned()
    }

    #[test]
    fn mermaid_same_day_points_get_distinct_date_based_labels() {
        let mut r = exact();
        r.series = vec![
            point("2026-01-02T01:00:00Z", "feat/a", 1.0, false),
            point("2026-01-02T05:00:00Z", "feat/b", 2.0, false),
            point("2026-01-03T05:00:00Z", "feat/c", 3.0, true),
            point("2026-01-03T06:00:00Z", "feat/d", 4.0, true),
        ];
        let chart = mermaid_block(&project_markdown(&r));
        let line = chart.lines().find(|l| l.contains("x-axis")).unwrap_or("");
        assert!(
            line.contains(
                "x-axis [\"2026-01-02 #1\", \"2026-01-02 #2\", \"2026-01-03 #1*\", \"2026-01-03 #2*\"]"
            ),
            "{line}"
        );
        assert!(!chart.contains("feat/"), "{chart}");
    }

    #[test]
    fn mermaid_title_says_at_least_not_floor_for_a_floor() {
        let md = project_markdown(&messy());
        assert!(
            md.contains("title \"Cumulative cost (USD, at least $3.75)\""),
            "{md}"
        );
        assert!(!md.contains("floor)"), "{md}");
    }

    #[test]
    fn mermaid_undated_only_floor_says_at_least_and_shows_last_point() {
        let mut r = exact();
        r.rows.push(row("scratch", None, 1, 10, 0.0));
        r.rows[2].cost = cost(0.5, 1);
        r.rows[2].unpriced_models = vec!["mystery-1".to_owned()];
        r.totals.cost = cost(4.25, 1);
        r.totals.unpriced_models = vec!["mystery-1".to_owned()];
        r.undated_branches = 1;
        let md = project_markdown(&r);
        assert!(md.contains("**at least $4.25**"), "{md}");
        assert!(
            md.contains("title \"Cumulative cost (USD, at least $3.75)\""),
            "{md}"
        );
        assert!(md.contains("dated branches only"), "{md}");
        let s = project_svg(&r);
        assert!(
            s.contains("<title>Cumulative cost: at least $3.75</title>"),
            "{s}"
        );
        assert!(s.contains("1 undated branch(es) not shown"), "{s}");
    }

    #[test]
    fn undated_exact_branch_makes_chart_at_least_with_last_point_value() {
        let mut r = exact();
        r.rows.push(row("scratch", None, 1, 10, 2.0));
        r.totals.cost = cost(5.75, 0);
        r.undated_branches = 1;
        let md = project_markdown(&r);
        assert!(md.contains("**$5.75**"), "{md}");
        assert!(
            md.contains("title \"Cumulative cost (USD, at least $3.75)\""),
            "{md}"
        );
        let s = project_svg(&r);
        assert!(
            s.contains("<title>Cumulative cost: at least $3.75</title>"),
            "{s}"
        );
        assert!(!s.contains("5.75"), "{s}");
    }

    #[test]
    fn overlap_that_is_a_floor_says_at_least_in_note_and_json() {
        let mut r = exact();
        r.totals.overlap = SharedOverlap {
            calls: 1,
            tokens: 100,
            cost: cost(0.5, 1),
        };
        let md = project_markdown(&r);
        assert!(md.contains("100 tokens, at least $0.50)"), "{md}");
        let v: serde_json::Value = serde_json::from_str(&project_json(&r)).unwrap();
        assert_eq!(v["overlap"]["costIsFloor"], true);
    }

    /// `feat/a` is dated but has 2 calls with no usable timestamp; `scratch` is wholly undated.
    fn with_undated_calls() -> ProjectReport {
        let mut r = exact();
        r.rows[0].undated_calls = 2;
        r.rows.push(row("scratch", None, 1, 10, 0.0));
        r.rows[2].undated_calls = 1;
        r.totals.undated_calls = 3;
        r.undated_branches = 1;
        r
    }

    #[test]
    fn markdown_names_dated_branches_whose_date_may_be_early() {
        let md = project_markdown(&with_undated_calls());
        assert!(
            md.contains(
                "- 3 call(s) have no usable timestamp. These branches may end later than their \
                 last call shown, because those calls could not be dated: `feat/a` (2 call(s))"
            ),
            "{md}"
        );
        assert!(!md.contains("`scratch` (1 call(s))"), "{md}");
        assert!(!project_markdown(&exact()).contains("no usable timestamp"));
    }

    #[test]
    fn json_carries_undated_call_counts_on_branches_and_top_level() {
        let v: serde_json::Value =
            serde_json::from_str(&project_json(&with_undated_calls())).unwrap();
        assert_eq!(v["undatedCalls"], 3);
        assert_eq!(v["branches"][0]["undatedCalls"], 2);
        assert_eq!(v["branches"][1]["undatedCalls"], 0);
        assert_eq!(v["branches"][2]["undatedCalls"], 1);
    }

    #[test]
    fn svg_footer_mentions_undated_calls_only_when_non_zero() {
        let s = project_svg(&with_undated_calls());
        assert!(
            s.contains("3 call(s) with no usable timestamp, branch dates may be early"),
            "{s}"
        );
        assert!(!project_svg(&exact()).contains("no usable timestamp"));
    }

    /// Parse `(viewBox height, height attribute, footer-line baselines)` from the SVG text.
    fn svg_geometry(s: &str) -> (f64, f64, Vec<f64>) {
        let after = |key: &str| s.split(key).nth(1).unwrap_or_default().to_owned();
        let first = |text: String| text.split('"').next().unwrap_or_default().to_owned();
        let vb_h = first(after("viewBox=\""))
            .split(' ')
            .nth(3)
            .and_then(|h| h.parse().ok())
            .unwrap_or(f64::NAN);
        let attr_h = first(after(" height=\"")).parse().unwrap_or(f64::NAN);
        let ys = s
            .lines()
            .filter(|l| l.starts_with("<text class=\"t\" x=\"72.0\"") && !l.contains("text-anchor"))
            .filter_map(|l| l.split(" y=\"").nth(1))
            .filter_map(|y| y.split('"').next()?.parse().ok())
            .collect();
        (vb_h, attr_h, ys)
    }

    fn with_all_footers() -> ProjectReport {
        let mut r = with_undated_calls();
        r.series[0].is_floor = true;
        r
    }

    #[test]
    fn every_footer_line_is_inside_the_view_box() {
        let s = project_svg(&with_all_footers());
        for text in [
            "hollow = floor",
            "undated branch(es) not shown",
            "no usable timestamp",
        ] {
            assert!(s.contains(text), "missing footer {text}: {s}");
        }
        let (vb_h, attr_h, ys) = svg_geometry(&s);
        assert_eq!(ys.len(), 3, "{s}");
        assert!((vb_h - attr_h).abs() < f64::EPSILON, "{s}");
        for y in ys {
            assert!(y < vb_h, "footer baseline {y} outside viewBox {vb_h}: {s}");
        }
    }

    #[test]
    fn footer_canvas_fits_any_line_count_and_small_reports_keep_360() {
        let (vb_h, _, _) = svg_geometry(&project_svg(&exact()));
        assert!((vb_h - 360.0).abs() < f64::EPSILON);
        for n in 1..12_u8 {
            let h = footer_canvas_height(usize::from(n));
            let last = 332.0 + 16.0 * f64::from(n - 1);
            assert!(last < h, "n={n} last={last} h={h}");
            assert!(h >= 360.0);
        }
    }

    // ---- Overflowed totals (#96) ----

    /// `exact()` with the first branch and the project totals saturated.
    fn overflowed() -> ProjectReport {
        let mut r = exact();
        r.rows[0].totals_overflowed = true;
        r.totals.totals_overflowed = true;
        r.totals.usage.input = u64::MAX;
        r
    }

    #[test]
    fn markdown_without_overflow_never_mentions_it() {
        for r in [exact(), messy()] {
            let md = project_markdown(&r);
            assert!(!md.contains("verflow"), "{md}");
        }
    }

    #[test]
    fn markdown_overflow_is_stated_and_totals_and_rows_are_floors() {
        let md = project_markdown(&overflowed());
        assert!(
            md.contains("**Totals overflowed and are unreliable**"),
            "{md}"
        );
        assert!(md.contains("Project total: **at least $3.75**"), "{md}");
        assert!(md.contains("| at least $1.25 |"), "{md}");
        assert!(md.contains("| $2.50 |"), "{md}");
        assert!(
            md.contains("title \"Cumulative cost (USD, at least $3.75)\""),
            "{md}"
        );
    }

    #[test]
    fn markdown_row_overflow_alone_labels_the_project_unreliable() {
        let mut r = exact();
        r.rows[1].totals_overflowed = true;
        let md = project_markdown(&r);
        assert!(
            md.contains("**Totals overflowed and are unreliable**"),
            "{md}"
        );
        assert!(md.contains("**at least $3.75**"), "{md}");
    }

    #[test]
    fn json_overflow_flag_appears_only_when_set() {
        let plain = project_json(&exact());
        assert!(!plain.contains("verflow"), "{plain}");
        let j = project_json(&overflowed());
        assert!(j.contains("\"totalsOverflowed\": true,"), "{j}");
        assert!(j.contains("\"totalCostIsFloor\": true"), "{j}");
        assert!(j.contains("\"totalTokensIsFloor\": true"), "{j}");
        let branch = j.lines().find(|l| l.contains("feat/a")).unwrap_or_default();
        assert!(branch.contains("\"totalsOverflowed\": true"), "{j}");
        assert!(branch.contains("\"costIsFloor\": true"), "{j}");
        let other = j.lines().find(|l| l.contains("feat/b")).unwrap_or_default();
        assert!(!other.contains("verflow"), "{j}");
        assert!(serde_json::from_str::<serde_json::Value>(&j).is_ok(), "{j}");
    }

    #[test]
    fn svg_overflow_is_labelled_and_exact_svg_is_unchanged() {
        assert!(!project_svg(&exact()).contains("verflow"));
        let s = project_svg(&overflowed());
        assert!(
            s.contains("<title>Cumulative cost: at least $3.75</title>"),
            "{s}"
        );
        assert!(s.contains("totals overflowed, unreliable"), "{s}");
        let (vb_h, attr_h, ys) = svg_geometry(&s);
        assert!((vb_h - attr_h).abs() < f64::EPSILON, "{s}");
        assert!(ys.iter().all(|y| *y < vb_h), "{s}");
    }
}
