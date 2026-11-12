# CLI reference

```text
branch-tally <COMMAND>
```

| Command | Does |
| ------- | ---- |
| [`report`](#branch-tally-report) | Report Claude Code token usage for a git branch. |
| [`record`](#branch-tally-record) | Append this branch's attributed usage to the repository ledger (write only). |
| [`project`](#branch-tally-project) | Report project-wide totals and the cost-over-time series from the ledger. |
| [`capture`](#branch-tally-capture) | Record this session's branch from a Claude Code hook payload on stdin. Run by the [capture plugin](integration/claude-code.md), not by hand. |
| [`prices update`](#branch-tally-prices-update) | Refresh the per-user price cache from the LiteLLM model price list. |
| [`prices show`](#branch-tally-prices-show) | Show which price table reports would use, with its source, date and path. |
| [`config show`](#branch-tally-config-show) | Show the config file path, whether it was found, and each effective value and its source. |
| [`config init`](#branch-tally-config-init) | Write a config file holding the built-in defaults. Never overwrites an existing file. |
| [`update`](#branch-tally-update) | Update branch-tally in place from a GitHub release. `upgrade` is an alias. |
| `help [COMMAND]` | Print the help for the tool or a command. |

Global options:

| Option | Does |
| ------ | ---- |
| `-h`, `--help` | Print help. Every command and subcommand accepts it. |
| `-V`, `--version` | Print the version, such as `branch-tally 0.6.1`. A build from source prints `0.0.0-dev`. |

## Exit codes

| Code | Meaning |
| ---- | ------- |
| `0` | Success. |
| `1` | The command failed. A one-line message on stderr says why: no records for the branch, a bad config file, a missing ledger, a `gh` failure, and so on. |
| `2` | The command line is wrong: an unknown flag, a bad value, two conflicting flags, or a flag that needs another one. A usage message goes to stderr. |

## Output streams

The report, JSON and command results go to **stdout**, written as UTF-8 with LF line endings on
every platform. Warnings and notes go to **stderr**, so you can redirect the report to a file and
still see them. Notes and warnings carry counts, model ids, branch names and paths only, never
transcript content.

## `branch-tally report`

```text
branch-tally report [OPTIONS] [BRANCH]
```

Reports the token usage and estimated cost for `BRANCH` (default: the current branch).

| Option | Default | Does |
| ------ | ------- | ---- |
| `[BRANCH]` | current branch | The branch to report on. |
| `--repo <PATH>` | `.` | The repository to report on. Any directory inside its main checkout or one of its worktrees works. |
| `--mode <MODE>` | `claim` | Attribution mode: `claim`, `strict` or `timeline`. See [Attribution modes](#attribution-modes). |
| `--strict` | | Same as `--mode strict`. |
| `--timeline` | | Same as `--mode timeline`. |
| `--work-branch <REGEX>` | see below | Timeline mode only: a regex naming the work branches. |
| `--format <FORMAT>` | `md` | Output format: `md` or `json`. See [Output formats](#output-formats). |
| `--md` | | Same as `--format md`. |
| `--json` | | Same as `--format json`. |
| `--pr [<N>]` | | Splice the table into pull request `N`, or this branch's open PR when `N` is omitted. See [Updating a pull request](#updating-a-pull-request). |
| `--dry-run` | | With `--pr`, print the new PR body without editing the PR. |
| `--cost` | on | Show the estimated-cost column. Undoes `cost = false` in the config file. |
| `--no-cost` | | Omit the estimated-cost column from the Markdown table. |
| `--redact` | off | Hide identifiers in the output. See [Redaction](#redaction). |
| `--no-redact` | | Show identifiers. Undoes `redact = true` in the config file. |
| `--keep-names` | | With redaction, keep real agent and skill names; ids and times are still hidden. |
| `--redact-names` | on | With redaction, replace agent and skill names with generic labels. Undoes `redact-names = false` in the config file. |
| `--config <PATH>` | user config path | Read this config file instead of the default location. It must exist. |
| `--from-ledger` | | Report from the [ledger](ledger.md) instead of the live transcripts. Works with no transcripts. |
| `--with-live` | | With `--from-ledger`, also read this machine's live transcripts. The ledger wins where both have a call, so nothing is counted twice. |
| `--ledger <PATH>` | `.tally/ledger.jsonl` | The ledger file, used with `--from-ledger`. The default is at the root of the current checkout. |

Each flag that has a [config file](config.md) key overrides it for one run. Precedence is flag,
then config file, then built-in default.

### Conflicting flags

These are command-line errors (exit 2):

- a shorthand together with its long form, even when they agree: `--strict` or `--timeline` with
  `--mode`, `--md` or `--json` with `--format`;
- `--strict` with `--timeline`, and `--md` with `--json`;
- both halves of an on/off pair: `--cost` with `--no-cost`, `--redact` with `--no-redact`,
  `--keep-names` with `--redact-names`;
- `--with-live` without `--from-ledger`;
- `--work-branch` when the effective mode is not timeline. The mode can come from `--mode
  timeline`, `--timeline` or the config file's `mode = "timeline"`. A `work-branch` set only in
  the config file is ignored outside timeline mode rather than being an error.

A few flags are accepted but have no effect in some combinations:

- `--dry-run` without `--pr` does nothing.
- `--ledger` without `--from-ledger` does nothing.
- `--pr` with JSON output (`--json`, `--format json` or `format = "json"` in the config file)
  prints the JSON and does not touch the PR. `--pr` only works with Markdown.
- `--no-cost` affects the Markdown table only. JSON output always includes costs.
- `--keep-names` and `--redact-names` do nothing unless redaction is on.

### Attribution modes

A transcript stamps each API call with the git branch the session was on. That stamp is often not
the branch the work was for: a session switches branches, a subagent in a worktree stamps the
worktree's throwaway branch, or a call carries no branch at all. The mode decides how branch-tally
assigns calls to branches.

**`claim` (the default).** Starts from each call's stamp, then:

- places a session's own call that has no stamp, or a `worktree-agent-…` stamp, on the branch the
  session's [hook captures](captures.md) name both before and after that call (*captured hook
  data*). A call with captures on one side only is not placed this way. This needs the capture
  plugin, and wins over the worktree and origin recovery below;
- recovers the branch of a subagent call stamped `worktree-agent-…` from the worktree it ran in
  (*worktree recovery*), and of an unstamped call read from a worktree's transcripts from that
  worktree (*origin recovery*), using `git worktree list`, so the worktree must still exist;
- assigns subagent calls to the branch of the session that dispatched them (*dispatcher roll-up*);
- counts a whole session toward a branch when that branch is the only work branch the session
  touched (*session claim*). A work branch here is one starting with `feat/`, `feature/`, `fix/`,
  `bugfix/`, `hotfix/`, `chore/` or `release/`. So a session that started on `main` and then moved
  to `feat/x` counts entirely toward `feat/x`.

**`strict`.** Keeps only the calls whose stamp is the branch: no session claim, no captured hook
data, no worktree recovery and no dispatcher roll-up. This is the most literal filter, and it
usually counts less than `claim`.

**`timeline`.** Follows each session's branch over time instead of giving it one branch. A call
stamped with a work branch stays there. A call stamped with something else (the default branch,
a `worktree-agent-…` branch) goes to the work branch the session was last on, or, before the
session's first work-branch call, to the first one it moves to. Subagent calls go to the
dispatcher's branch at the moment of dispatch. A call with no usable timestamp, or two work
branches at the same instant, is ambiguous: it keeps its raw stamp and is reported as such. There
is no worktree lookup in this mode, and hook captures are not used.

In timeline mode, a work branch is by default every branch except the repository's default branch
and `worktree-agent-*`. The default branch is read from `origin/HEAD`, then from git's
`init.defaultBranch`, and is otherwise assumed to be `main`; the footnote says which. A
`worktree-agent-*` branch is never a work branch. `--work-branch <REGEX>` replaces that rule. The
regex is matched anywhere in the branch name, as written, so anchor it with `^` and `$` to match
the whole name:

```sh
branch-tally report --timeline --work-branch '^(feat|fix)/'
```

Every heuristic that assigned calls is listed in the report's footnote with its call and token
counts, and in the JSON `heuristics` object. When calls were ambiguous, the report says the token
total is "at least" the figure shown.

### Output formats

**Markdown** (`--format md`, the default) is the table shown in
[Getting Started](getting-started.md#3-read-the-output), wrapped in `<!-- token-report:start -->`
and `<!-- token-report:end -->` markers. It has, in order:

1. the model table: calls, input, cache write (5-minute and 1-hour combined), cache read, output,
   thinking and estimated cost per model, then the total;
2. the session table: each session's id, where it ran (main checkout or worktree), its named
   agents, its UTC time span, calls and tokens (in millions);
3. a bold "Token total is at least …" line, when some calls were ambiguous;
4. a footnote: the branch and mode, session and subagent counts, the heuristics used, the
   work-branch rule in timeline mode, whether identifiers were redacted, the price table's date
   and source, unpriced models, and a reminder that costs are estimates at list API rates;
5. a **Subagent usage** section, when subagents did part of the work: one row per subagent and
   model with its workspace, plus how many subagent calls had no dispatch link;
6. a collapsed **Breakdown by skill / agent** table.

With `--no-cost`, the cost columns are left out.

**JSON** (`--format json`) is one object on stdout. Its top-level keys:

| Key | Holds |
| --- | ----- |
| `branch` | The branch reported on. |
| `mode` | `"claim"`, `"strict"` or `"timeline"`. |
| `redacted` | `true` when `--redact` was on. Absent otherwise. |
| `totalCalls` | API calls in the report. |
| `totalCostUSD` | The estimated total cost in US dollars. |
| `totalCostIsFloor` | `true` when the cost is a lower bound, for example because a model is unpriced. |
| `unpricedModels` | Model ids that had no price. |
| `priceTable` | `asOf` (the table's date) and `source`: `"bundled"`, `"cache"` or `"override"`. |
| `heuristics` | Calls and tokens per attribution heuristic (`sessionClaim`, `worktreeRecovery`, `originRecovery`, `dispatcherRollUp`, `timelineForward`, `timelineCarried`) and `ambiguous`, plus `isFloor`. `captured` is added only when [hook captures](captures.md) placed at least one call. Absent when no heuristic applied. |
| `subagents` | Subagent totals (calls, tokens, dispatched subagents, workspaces, worktrees, unattributed and unlinked counts, cost) and `rows`, one per subagent and model with token counts and cost. Present when subagents did part of the work. |
| `sessions` | One object per session: `id`, `origin` (`"main"` or the worktree), `first` and `last` timestamps, `calls`, `tokens` and `agents`. |
| `models` | One object per model id: `calls`, `input`, `cacheWrite5m`, `cacheWrite1h`, `cacheRead`, `output`, `thinking` and `costUSD` (`null` when unpriced). |

An abridged example, from a scrubbed test fixture:

```json
{
  "branch": "main",
  "mode": "claim",
  "totalCalls": 7,
  "totalCostUSD": 106.9266,
  "totalCostIsFloor": true,
  "unpricedModels": [
    "claude-mystery-9"
  ],
  "priceTable": {
    "asOf": "2026-11-09",
    "source": "bundled"
  },
  "sessions": [
    {
      "id": "s1",
      "origin": "main",
      "first": "2026-01-01T00:00:01.000Z",
      "last": "2026-01-01T00:00:07.000Z",
      "calls": 7,
      "tokens": 117337218,
      "agents": []
    }
  ],
  "models": {
    "claude-mystery-9": {
      "calls": 1,
      "input": 11111,
      "cacheWrite5m": 444,
      "cacheWrite1h": 55,
      "cacheRead": 3333,
      "output": 2222,
      "thinking": 0,
      "costUSD": null
    }
  }
}
```

### Updating a pull request

`--pr` puts the Markdown report into a pull request's description. It runs the
[GitHub CLI](https://cli.github.com/), so `gh` must be installed and signed in.

```sh
branch-tally report --pr --dry-run   # print the new PR body, change nothing
branch-tally report --pr             # write it to this branch's open PR
branch-tally report --pr 42          # write it to PR #42
```

Without a number, it looks up the open PR whose head is the branch. If the PR body already holds
the `<!-- token-report:start -->` and `<!-- token-report:end -->` markers, the text between them is
replaced. Otherwise a `## Token usage` section holding the report is appended to the body.

It tells you on stderr what happened: `Token usage table written to PR #N.`,
`PR #N already up to date.`, or, with `--dry-run`, the new body on stdout followed by
`--- dry run: PR #N not modified ---`. With no open PR for the branch it fails with
`No open PR found for branch '…'. Open the PR first, then re-run with --pr.` (exit 1).

To keep a place for the report in your PR template, add the markers where you want it:

```markdown
## Token usage

<!-- token-report:start -->
_Not yet recorded._
<!-- token-report:end -->
```

### Redaction

`--redact` hides identifiers in the report, for Markdown and JSON alike:

- session ids become `s1`, `s2`, … in order of each session's first call;
- agent and skill names become `agent-1`, `skill-1`, … (`main` is kept);
- subagent ids become `a1`, `a2`, …, and worktree names become `worktree-1`, …;
- timestamps are reduced to dates, and the session table's header reads `Span (UTC date)`.

Model ids, branch names, token counts and costs are kept. The output says it was redacted: the
footnote includes `identifiers redacted`, and the JSON has `"redacted": true`. `--no-cost` is a
separate switch.

`--keep-names` keeps the real agent and skill names while ids, worktree names and times stay
hidden.

Redaction applies to the report only. The [ledger](ledger.md#redaction-and-the-ledger) stores real
identifiers.

### What the report says on stderr

| Message | Meaning |
| ------- | ------- |
| `warning: no price for <model> in the price table (as of <date>); the total is a floor` | A model has no price. Its tokens are counted, its cost is `—`, and the total is "at least". |
| `note: skipped N transcript record(s): …` | Transcript lines that could not be used: malformed lines, records without a request id, or synthetic records. Counted, not hidden. |
| `note: skipped N capture line(s) in .tally/captures.jsonl: …` | Hook capture lines that were malformed, written by an unsupported version, or of an unknown kind. Counted, not hidden. Printed by `report` and `record`. |
| `note: git worktree list was unavailable, so worktree recovery was skipped` | `git worktree list` failed, so subagent calls in worktrees could not be recovered. |
| The ledger notes | See [Report from the ledger](ledger.md#report-from-the-ledger). |

### Common errors

| Message (stderr, exit 1) | Cause |
| ------------------------ | ----- |
| `No transcript records found for branch '…'. Transcripts are local and purged after cleanupPeriodDays (default 30).` | Nothing for this branch in this machine's transcripts. |
| `No Claude Code transcript directory at <path>` | `~/.claude/projects` does not exist on this machine. |
| `No ledger records found for branch '…'. …` | `--from-ledger` found no lines for the branch. |
| `no ledger at <path>` | `--from-ledger` and the ledger file does not exist. |
| `refusing to use <path>: it is …` | A default `.tally/` path is a symbolic link or the wrong kind of file. See [Symbolic links are refused](ledger.md#symbolic-links-are-refused). |
| `cannot read price table <path>: …` | The price override or cache could not be read. A refused `.tally/prices.toml` shows as `refusing to use …` instead; see [Which price table is used](#which-price-table-is-used). |
| `config file <path> …` | The config file is invalid. See [Config reference](config.md#errors). |

## `branch-tally record`

```text
branch-tally record [OPTIONS] [BRANCH]
```

Selects `BRANCH`'s usage (default: the current branch) exactly as `report` would and appends it to
the [ledger](ledger.md). It writes only the ledger file: no `git add`, no commit. It prints
`appended N, skipped M, branch B`.

| Option | Default | Does |
| ------ | ------- | ---- |
| `[BRANCH]` | current branch | The branch to record. |
| `--repo <PATH>` | `.` | The repository. |
| `--mode <MODE>` | `claim` | Attribution mode: `claim`, `strict` or `timeline`. |
| `--strict` | | Same as `--mode strict`. |
| `--timeline` | | Same as `--mode timeline`. |
| `--work-branch <REGEX>` | | Timeline mode only: a regex naming the work branches, as for `report`. |
| `--config <PATH>` | user config path | Read this config file instead of the default location. It must exist. |
| `--ledger <PATH>` | `.tally/ledger.jsonl` | The ledger file. The default is at the root of the current checkout. |

The same conflict rules as `report` apply to the mode flags and `--work-branch`. From the config
file, `record` uses `mode` and `work-branch` and ignores the rest.

`record` never creates `.tally/`. With the default ledger it writes only when the checkout already
has a `.tally/` directory (see [Opt the repository in](ledger.md#opt-the-repository-in)).
Otherwise it writes and creates nothing, and exits 0 with this on stdout instead of the
`appended` line:

```text
no .tally/ directory: nothing recorded (create it to opt in)
```

An explicit `--ledger <PATH>` is written whether or not `.tally/` exists, and a symbolic link
there is followed.

| Message (stderr, exit 1) | Cause |
| ------------------------ | ----- |
| `refusing to use <path>: it is …` | `.tally`, `.tally/ledger.jsonl` or `.tally/captures.jsonl` is a symbolic link or the wrong kind of file. Nothing is written. See [Symbolic links are refused](ledger.md#symbolic-links-are-refused). |

`record` also fails with the transcript errors listed under [Common errors](#common-errors).

## `branch-tally project`

```text
branch-tally project [OPTIONS]
```

Reads the [ledger](ledger.md) only (never the live transcripts) and prints the project total, a
per-branch table and a Mermaid cost-over-time chart, or the same data as JSON. Each API call is
counted once in the total, even when two branches claim it. Any cost that leaves out an unpriced
model is shown as a floor, "at least $X". What the report holds and how to read it is in
[Project Cost Over Time](project.md).

| Option | Default | Does |
| ------ | ------- | ---- |
| `--repo <PATH>` | `.` | The repository, used to find the default ledger and the price table. |
| `--ledger <PATH>` | `.tally/ledger.jsonl` | The ledger file. The default is at the root of the current checkout. |
| `--format <FORMAT>` | `md` | Output format: `md` or `json`. See [Output formats](project.md#output-formats). |
| `--md` | | Same as `--format md`. |
| `--json` | | Same as `--format json`. |
| `--config <PATH>` | user config path | Read this config file instead of the default location. It must exist. |
| `--export-graph <PATH>` | | Also write the chart as an SVG file to `PATH`. The report still prints. See [SVG](project.md#svg). |

`--md` or `--json` together with `--format`, and `--md` with `--json`, are command-line errors
(exit 2), as for `report`. The config file's `format` key sets the default format; a flag
overrides it. The config file's `mode` is used as `report --from-ledger` uses it; `project` has no
mode flags. See [What it reads](project.md#what-it-reads).

`--export-graph` writes the file whole or not at all, and replaces an existing file. It does not
create the parent directory. The SVG is written before the report prints, so when it fails nothing
is printed on stdout.

| Message (stderr, exit 1) | Cause |
| ------------------------ | ----- |
| `no ledger at <path>` | The ledger file does not exist, or the checkout has no `.tally/` directory. |
| `refusing to use <path>: it is …` | The default `.tally/ledger.jsonl`, or `.tally` itself, is a symbolic link or the wrong kind of file. It is not read. `--ledger <PATH>` is exempt. See [Symbolic links are refused](ledger.md#symbolic-links-are-refused). |
| `cannot read price table <path>: …` | The price override or cache could not be read. A refused `.tally/prices.toml` or `.tally` prints the `refusing to use …` line above instead. See [Which price table is used](#which-price-table-is-used). |
| `cannot place the default ledger: <path> is not inside a git checkout …; pass --ledger <path>` | Outside a git checkout there is no default ledger, so `--ledger` is required. |
| `cannot write graph <path>: <error>` | `--export-graph` could not write the file, for example because its directory does not exist. |
| `config file <path> …` | The config file is missing (with `--config`) or invalid. See [Config reference](config.md#errors). |

A ledger with no lines is not an error: the report says `No branches recorded in the ledger.` The
ledger notes on stderr are the same as for `report --from-ledger`; see
[Report from the ledger](ledger.md#report-from-the-ledger).

## `branch-tally capture`

```text
branch-tally capture
```

Run by the [capture plugin](integration/claude-code.md)'s hooks (`SessionStart`, `SubagentStop`,
`Stop`), not by hand. It takes no options. It reads one hook payload from stdin and appends an
id-only event to `.tally/captures.jsonl` in the **main checkout**, so a worktree session's captures
land in one place. The event holds the session id, the agent id, the branch, the event kind and a
timestamp. No path and no message content is stored or printed. The file and how reports use it are
described in [Hook captures](captures.md).

- The branch is the current branch of the payload's `cwd` (the process directory when the payload
  has none). A detached HEAD records nothing.
- For `SubagentStop`, `worktree_branch` is the branch of the worktree directory named
  `agent-<agent id>` in `git worktree list` (compared case-insensitively). Failing that, it is the
  branch of the payload's own checkout when that is a linked worktree. Otherwise it is absent. It
  is never guessed.
- `SessionEnd` and `SubagentStart` payloads are accepted and ignored.
- It writes only when the main checkout already has a `.tally/` directory, and never creates it
  (see [Opt the repository in](ledger.md#opt-the-repository-in)). Without one it writes nothing,
  prints nothing and exits 0.
- It prints nothing on success. On failure (malformed payload, not a git checkout, a detached HEAD,
  a git or write error) it prints one short fixed line to stderr and exits 1. The hook command ends
  in `; exit 0`, so a failure never blocks the session; that also covers a command-line usage error,
  which exits 2.
- A symbolic link at `.tally` or `.tally/captures.jsonl`, or a captures path that is not a regular
  file, is refused, not followed. It writes nothing, and exits 1 with this line on stderr, which
  names no path:

  ```text
  capture: refusing to write: .tally/ or its captures file is a symlink or not a regular file
  ```

## `branch-tally prices update`

```text
branch-tally prices update
```

Downloads [LiteLLM](https://github.com/BerriAI/litellm)'s public
`model_prices_and_context_window.json` from GitHub, keeps the Anthropic chat models, and writes
them to the per-user price cache. It replaces the cache only when the download and the conversion
both succeed, so a failed update leaves a good cache in place. It prints the path written, the
table's date, the model count, and any models skipped for lacking a usable price.

It sends no account or identifying data, only a `branch-tally/<version>` User-Agent. Despite the
name, it does not update branch-tally itself; [`update`](#branch-tally-update) does that.

The cache file is `branch-tally/prices.toml` under:

| OS | Directory |
| -- | --------- |
| Windows | `%LOCALAPPDATA%` |
| macOS | `$HOME/Library/Caches` |
| Linux and others | `$XDG_CACHE_HOME` when set to an absolute path, else `$HOME/.cache` |

## `branch-tally prices show`

```text
branch-tally prices show [--repo <PATH>]
```

Prints which price table reports in this repository would use:

```text
source: bundled
as of: 2026-11-09
models: 23
path: (built into the binary)
```

`--repo <PATH>` (default `.`) is the repository whose `.tally/prices.toml` override is checked.

### Which price table is used

The first of these that exists is used whole, with no per-model merging:

1. `.tally/prices.toml` in the repository's main checkout (the override, `source: override`);
2. the per-user cache written by `prices update` (`source: cache`);
3. the table built into the binary (`source: bundled`).

A model id is priced by the longest model key that is a prefix of it, so `claude-fable-5-1` also
prices `claude-fable-5-1[1m]`. A model that matches nothing is unpriced, and the report says so.
A malformed override or cache stops the report with a message naming the file; it is never
skipped silently.

The override is read only from a real `.tally/` directory, and never through a symbolic link (see
[Symbolic links are refused](ledger.md#symbolic-links-are-refused)). With no `.tally/` directory
there is no override, and the next table in the list is used. When `.tally` or
`.tally/prices.toml` is a link, or is the wrong kind of file, `report`, `project` and `prices show`
stop with exit 1 and the refusal, which names the path and the reason:

```text
refusing to use <main checkout>/.tally/prices.toml: it is not a regular file
```

A linked `.tally` ends `it is a symbolic link`, and a `.tally` that is a file ends
`it is not a directory`.

Replace the link with a real file, or remove it.

The override and the cache share one TOML format:

| Key | Required | Meaning |
| --- | -------- | ------- |
| `as_of` | yes | The date the prices were true, quoted: `"YYYY-MM-DD"`. |
| `[cache_write]` `five_minute` | no, default `1.25` | 5-minute cache writes cost this multiple of the input rate. |
| `[cache_write]` `one_hour` | no, default `2.0` | 1-hour cache writes cost this multiple of the input rate. |
| `[cache_read]` `default_factor` | no, default `0.1` | Cache reads cost this fraction of the input rate. |
| `[models."<id>"]` `input` | yes | Input price, dollars per million tokens. |
| `[models."<id>"]` `output` | yes | Output price, dollars per million tokens. |
| `[models."<id>"]` `cache_read` | no | Cache-read price for this model, dollars per million tokens. |
| `[models."<id>"]` `cache_write_5m` | no | This model's multiplier, replacing `five_minute`. |
| `[models."<id>"]` `cache_write_1h` | no | This model's multiplier, replacing `one_hour`. |

A minimal override:

```toml
as_of = "2026-11-09"

[models."claude-opus-5"]
input = 5.0   # $ per million tokens
output = 25.0
```

The bundled table, with the schema in its header, is
[`crates/branch-tally-core/data/prices.toml`](https://github.com/IAmSkweetis/branch-tally/blob/main/crates/branch-tally-core/data/prices.toml).

## `branch-tally config show`

```text
branch-tally config show [--config <PATH>]
```

Prints the config file's path, whether it was found, and each setting's effective value with its
source (`config file` or `default`):

```text
config file: /home/you/.config/branch-tally/config.toml
found: no
mode = claim (default)
work-branch = (not set) (default)
format = md (default)
cost = true (default)
redact = false (default)
redact-names = true (default)
```

A `work-branch` that is set while the mode is not timeline carries
`[inactive: mode is not timeline]`. `--config <PATH>` reads that file instead of the default
location; it must exist. See the [Config reference](config.md).

## `branch-tally config init`

```text
branch-tally config init [--path <PATH>]
```

Writes a starter config file holding the built-in defaults to `--path <PATH>`, or to the default
location that `config show` prints. It creates missing directories, prints `wrote <path>`, and
never overwrites an existing file (exit 1). See the [Config reference](config.md#config-init).

## `branch-tally update`

```text
branch-tally update [OPTIONS]
branch-tally upgrade [OPTIONS]
```

Replaces the running `branch-tally` binary with a release from the
[releases page](https://github.com/IAmSkweetis/branch-tally/releases). `upgrade` is an alias and
behaves identically. It works on a binary installed by `install.sh`, `install.ps1` or a
[download by hand](installation/manual-download.md), on the six
[release targets](installation/).

| Option | Does |
| ------ | ---- |
| `--check` | Only report whether a different version is available. Installs nothing. |
| `--version <vX.Y.Z>` | Install this release instead of the latest. The leading `v` is optional. It may be older than the running version, so this is also how to downgrade. |
| `--yes` | Do not ask for confirmation. |
| `--force` | Reinstall even when already on the target version. |

In order, it:

1. Refuses a development build (see [Refusals](#refusals)).
2. Unless `--check` is given, refuses when it cannot write to the directory holding the binary.
3. Picks the target version: `--version`, or else the latest release, learned from that release's
   `SHA256SUMS`.
4. Stops if there is nothing to do: already on the `--version` asked for, or, without `--version`,
   already on the latest release or newer. `--force` carries on and reinstalls.
5. Asks `Update branch-tally vA -> vB? [y/N]` on stderr, unless `--yes` is given. Only `y` or `yes`
   proceeds. When stdin is not a terminal it cannot ask, so it declines and says to pass `--yes`.
6. Downloads the release's `SHA256SUMS` and the archive for this platform, and checks the archive
   against it. A checksum mismatch, or an archive that `SHA256SUMS` does not list, is an error and
   nothing is installed.
7. Takes only the binary out of the archive and moves it over the running executable in one step.
   A symlink is followed, so the file it points at is replaced. On Windows the running `.exe` is
   renamed to `branch-tally.exe.old` first, because a running program cannot be overwritten. The
   `.old` file is deleted the next time `update` runs.

A failure at any step leaves the installed binary as it was. To use `update` in a script or CI job,
pass `--yes`.

### What it prints

One line on stdout when it succeeds, or on stderr when it fails:

| Situation | Line | Exit |
| --------- | ---- | ---- |
| Updated | `updated branch-tally v0.6.0 -> v0.7.0` | `0` |
| Already on the target | `branch-tally is up to date (v0.6.0)` | `0` |
| You answered no, or stdin is not a terminal | `update v0.6.0 -> v0.7.0 declined; nothing changed` | `0` |
| `--check` found another version | ``update available: v0.6.0 -> v0.7.0; run `branch-tally update` to install it`` | `0` |
| A refusal or an error | see below | `1` |
| A bad command line | a usage message | `2` |

`--check` with `--version` compares the version you named with the running one. It does not check
that a release with that tag exists, so it reports `update available` for any version that differs.

### Refusals

`update` refuses, exits 1 and prints the install one-liners in its message when:

- the binary is a development build, version `0.0.0-dev`, which is what
  [a build from source](installation/from-source.md) or `cargo run` produces. Install a release
  instead, or update your clone and rebuild it;
- the directory holding the binary is not writable, for example a system directory you need
  `sudo` for. Rerun the installer instead, which can install into a directory you choose.

### Errors

| Message starts with | Cause |
| ------------------- | ----- |
| `not a valid version` | `--version` is not `X.Y.Z` or `vX.Y.Z`. |
| `release not found` | No release has that tag, or the mirror has no latest release. |
| `no release is built for target` | This binary was built for a platform with no prebuilt release. |
| `download failed` | A network or read error. URLs are left out of the message. |
| `<archive> is not listed in SHA256SUMS`, `malformed checksum`, `checksum mismatch` | The download could not be verified. Nothing was installed. |
| `install failed` | The new binary could not be extracted or moved into place. The old one is kept. |

### Release mirror

`BRANCH_TALLY_INSTALL_BASE_URL`, the variable the installers read, points `update` at a mirror with
the same layout as the releases page (`<base>/latest/download/…` and `<base>/download/<tag>/…`).
The default is `https://github.com/IAmSkweetis/branch-tally/releases`. A `file://` URL naming a
local directory also works. Any other scheme, including plain `http://`, is refused before any
network call.

`update` sends no account or identifying data, only a `branch-tally/<version>` User-Agent.
