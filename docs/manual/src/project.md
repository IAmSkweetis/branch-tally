# Project Cost Over Time

`branch-tally report` answers "what did this branch cost?". `branch-tally project` answers "what
has the whole project cost so far, and how did it grow?". It reads the [ledger](ledger.md) and
prints:

- the **project total**: calls, tokens and estimated cost, with each API call counted once;
- a **per-branch table**: one row for each branch the ledger holds, with its last call date;
- **notes** on anything that makes a number uncertain;
- a **cost-over-time chart**: the cumulative cost, one point per branch, as a Mermaid block.

```sh
branch-tally project                                     # Markdown, from .tally/ledger.jsonl
branch-tally project --json                              # the same data as JSON
branch-tally project --export-graph docs/project-cost.svg   # also write the chart as SVG
```

The full option list is in the [CLI reference](cli.md#branch-tally-project).

## What it reads

`project` reads the ledger only. It never reads the live transcripts, so it works the same on a
fresh clone as on the machine that did the work. The default ledger is `.tally/ledger.jsonl` at the
root of the git checkout that holds the current directory, or the `--repo` directory. `--ledger
<PATH>` reads another file. A missing ledger is an error (`no ledger at <path>`, exit 1), not an
empty report.

That makes the project total **the total of what was recorded**. A branch whose usage nobody ran
[`record`](ledger.md#record) for, or whose ledger lines were never committed and merged, is not in
it. Record every branch, on every machine that worked on it, if you want the total to cover the
whole project.

Prices are applied when you run `project`, from the price table for the `--repo` repository,
exactly as for `report --from-ledger`. See
[Which price table is used](cli.md#which-price-table-is-used).

The ledger notes that `report --from-ledger` prints on stderr (skipped lines, conflicting lines,
mixed modes) are printed by `project` too. They are described in
[Report from the ledger](ledger.md#report-from-the-ledger).

## The per-branch table

There is one row for each branch that `record` was run for: the ledger's `recorded_for` field. A
row's numbers are what `branch-tally report <branch> --from-ledger` shows for that branch.

| Column | Holds |
| ------ | ----- |
| Branch | The branch `record` was run for. |
| Last call | The date of the latest API call in the branch's ledger lines, or `undated` when none of them has a timestamp. |
| Calls | API calls recorded for the branch, shared calls included. |
| Tokens | All tokens in those calls: input, cache writes, cache reads, output and thinking. |
| Cost | The estimated cost at list API rates, or `at least $X` when a model had no price. |

Rows are in order of their last call, oldest first. Branches with the same last call are ordered
by name, and undated branches come last.

## Shared calls: why the rows can add up to more than the total

In claim mode, one API call can count toward two branches: for example, a session that started on
one feature branch and moved to another. `record` then writes the call once for each branch it
was recorded for (see [Record](ledger.md#record)).

`project` keeps both views honest:

- each **row** counts every call recorded for its branch, shared calls included, so the row
  matches that branch's own report;
- the **project total** counts each API call once, however many branches claim it.

So the rows can sum to more than the total. When that happens, a note under the table says by how
much:

```text
- Shared calls: 1 call(s) (3,240,000 tokens, $3.50) were counted more than once in the rows
  because two or more branches claim them. The project total counts each once, so the rows may
  sum to more than the total.
```

(The real note is a single line; it is wrapped here to fit the page.) In JSON, the same figures
are in `overlap`.

## Floors: "at least $X"

A cost that leaves something out is shown as a floor, a lower bound, never as an exact figure:

- A **row** is `at least $X` when one of its models has no price in the price table. Its tokens
  are counted, but the unpriced model's cost is not in `$X`.
- The **project total** is `at least $X` when any figure in the report is a floor: the total, any
  row or any chart point. A note names the unpriced models.

To make a floor exact, give the model a price: run `branch-tally prices update` if the model is
newer than your price table, or add it to a `.tally/prices.toml` override. See
[Which price table is used](cli.md#which-price-table-is-used).

Token counts can be floors too. When some calls could not be placed on a branch with certainty, a
note says how many, and that "Token totals are at least what is shown". In JSON,
`totalTokensIsFloor` is `true`.

When attribution heuristics such as session claim or dispatcher roll-up assigned calls to a
branch, a note lists each one with its call and token counts. Those calls are counted, but their
branch was inferred rather than stamped. The heuristics are described in
[Attribution modes](cli.md#attribution-modes).

## The cost-over-time chart

The chart has one point per dated branch, in the same order as the table. Each point's height is
the cumulative cost: that branch's cost plus every earlier branch's. Its label is the branch's
last call date. Each label is unique, so when two branches have their last call on the same day,
their labels still tell the points apart. The chart shows dates only; branch names are in the
table.

A shared call is added once, at the earliest branch that claims it. So the chart never counts a
call twice, and its last point is the total cost of all the dated branches.

Two things the chart leaves out, and says so:

- **Undated branches.** A branch with no timestamp has no place on a time axis. It is in the table
  and the total, but not on the chart, and a note says how many were left off. The chart then
  covers the dated branches only, and its last point is lower than the project total.
- **The scale of time.** Points are evenly spaced in date order. The distance between two points
  does not show how much time passed between them.

**Floor points.** Once a branch is a floor, every point from there on is a floor too, because each
point includes every earlier branch. The Markdown chart marks a floor point with `*` on its label,
says "at least" in its title, and adds a line under the chart explaining the `*`. The SVG draws a
floor point as a hollow circle, with a dashed segment leading into it.

## Example

The output below comes from a small synthetic ledger, made up for this page. It has four branches:
`fix/typo` shares one call with `feat/login`, `feat/search` used a model with no price, and
`chore/old-notes` has no timestamps. The Mermaid chart block that follows the notes is left out
here.

```markdown
## Project cost

Project total: **at least $21.70** · 8 call(s) · 25,920,000 tokens (25.9M)

| Branch | Last call | Calls | Tokens | Cost |
| --- | --- | ---: | ---: | ---: |
| `feat/login` | 2026-03-02 | 4 | 12,960,000 | $11.20 |
| `fix/typo` | 2026-03-09 | 2 | 6,480,000 | $7.00 |
| `feat/search` | 2026-03-20 | 2 | 6,480,000 | at least $3.50 |
| `chore/old-notes` | undated | 1 | 3,240,000 | $3.50 |

- Shared calls: 1 call(s) (3,240,000 tokens, $3.50) were counted more than once in the rows because two or more branches claim them. The project total counts each once, so the rows may sum to more than the total.
- 1 model(s) unpriced, total is a floor: `claude-mystery-9`
- Attributed by heuristic: session claim 1 call(s) / 3,240,000 tokens
- 1 branch(es) have no timestamp and are left off the chart.
```

Reading it:

- The rows sum to $25.20 and 9 calls. The total is $21.70 and 8 calls, because the shared call is
  counted once.
- The total is "at least", because `feat/search` used `claude-mystery-9`, which has no price.
- The chart has three points: $11.20, $14.70 and at least $18.20. The last is a floor, marked
  with `*` (hollow in the SVG).
- The chart ends at $18.20, not $21.70: `chore/old-notes` ($3.50) is undated, so it is in the
  total but not on the chart.
- `fix/typo` adds $3.50 to the chart, not its row's $7.00, because its shared call was already
  counted at `feat/login`.

## Output formats

### Markdown

Markdown (`--format md`) is the default. It is the `## Project cost` section shown above: the
total line, the table, the notes and the Mermaid `xychart-beta` block. With no branches in the
ledger, it says `No branches recorded in the ledger.` With no dated branches, it prints the table
and says `No dated branches, so there is no cost chart.`

The output holds branch names, dates, model ids and numbers only. `project` has no redaction
option: branch names appear in the table as they are in the ledger.

### JSON

`--json` (or `--format json`) prints one object on stdout:

| Key | Holds |
| --- | ----- |
| `totalCalls` | API calls in the project, each counted once. |
| `totalTokens` | Their tokens, each call counted once. |
| `totalTokensIsFloor` | `true` when some calls could not be placed with certainty, so token totals are a lower bound. |
| `totalCostUSD` | The estimated project cost in US dollars, each call counted once. |
| `totalCostIsFloor` | `true` when any figure in the report is a floor: the total, a branch or a series point. |
| `unpricedModels` | Model ids that had no price, across all branches. |
| `overlap` | The shared calls: `calls`, `tokens`, `costUSD` and `costIsFloor`. Counted in more than one branch, once in the totals. |
| `heuristics` | Calls and tokens per attribution heuristic, and `ambiguous`, as in `report`'s JSON, plus `isFloor`. Every key is present, with zeros where nothing applied. |
| `undatedBranches` | How many branches have no timestamp and are left off the series. |
| `branches` | One object per branch, in table order: `branch`, `lastCall` (the full timestamp, or `null`), `calls`, `tokens`, `costUSD`, `costIsFloor`, `unpricedModels` and `heuristics`. |
| `series` | One object per chart point, in date order: `date` (the full timestamp), `branch`, `cumulativeCostUSD` and `isFloor`. |

Dollar amounts are rounded to four decimal places. An abridged example from the synthetic ledger
above, with the `heuristics` objects left out:

```json
{
  "totalCalls": 8,
  "totalTokens": 25920000,
  "totalTokensIsFloor": false,
  "totalCostUSD": 21.7,
  "totalCostIsFloor": true,
  "unpricedModels": ["claude-mystery-9"],
  "overlap": { "calls": 1, "tokens": 3240000, "costUSD": 3.5, "costIsFloor": false },
  "undatedBranches": 1,
  "branches": [
    { "branch": "feat/login", "lastCall": "2026-03-02T13:00:00Z", "calls": 4, "tokens": 12960000, "costUSD": 11.2, "costIsFloor": false, "unpricedModels": [] },
    { "branch": "chore/old-notes", "lastCall": null, "calls": 1, "tokens": 3240000, "costUSD": 3.5, "costIsFloor": false, "unpricedModels": [] }
  ],
  "series": [
    { "date": "2026-03-02T13:00:00Z", "branch": "feat/login", "cumulativeCostUSD": 11.2, "isFloor": false },
    { "date": "2026-03-20T16:00:00Z", "branch": "feat/search", "cumulativeCostUSD": 18.2, "isFloor": true }
  ]
}
```

### SVG

`--export-graph <PATH>` also writes the chart as a standalone SVG file, 640 by 360 pixels. The
report still prints on stdout as usual, in whichever format you chose.

- The heading states the cumulative cost, with "at least" in front of it when it is a floor. When
  some branches are undated, the heading says the chart covers dated branches only.
- The axis shows the first and last dates. A floor point is a hollow circle, and the segment
  leading into it is dashed. A line under the chart explains this when there is a floor, and
  another says how many undated branches are not shown.
- Branch names are not printed on the chart. Each point has a tooltip with its branch, date and
  cumulative cost. Browsers don't show tooltips for an SVG embedded as an image, so open the file
  itself to see them.
- The colours follow the viewer's light or dark colour-scheme preference, so the chart stays
  readable on both. The file loads nothing from the network.

The file is written whole or not at all: a failed write leaves any existing file as it was. An
existing file at `PATH` is replaced. The parent directory must already exist. If the file can't be
written, `project` prints `cannot write graph <path>: <error>` on stderr, prints no report, and
exits 1.

## Put the chart in your README

`project` only writes files you name, and commits nothing. To show the chart in your repository's
README, write it to a file in the repository and commit it with the ledger.

**As an SVG image.** Write the chart, discarding the report:

```sh
mkdir -p docs
branch-tally project --export-graph docs/project-cost.svg > /dev/null
```

Then reference it from `README.md`:

```markdown
![Cumulative AI-agent cost of this project](docs/project-cost.svg)
```

Re-run the command, and commit the new file, whenever the ledger changes.

**As a Mermaid block.** GitHub renders Mermaid in Markdown, so you can paste the chart block into
the README instead. On Linux and macOS, this prints just the block:

```sh
branch-tally project | sed -n '/^```mermaid$/,/^```$/p'
```

If the chart has floor points, paste the line explaining `*` along with it, so a reader knows
those points are lower bounds.
