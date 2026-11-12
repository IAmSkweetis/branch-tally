# Config reference

A config file holds your standing defaults for `report`'s options, so you don't have to pass the
same flags every time. It is optional: with no file, every setting has a built-in default.

## Precedence

For each setting, the first of these that is set wins:

1. a command-line flag;
2. the config file;
3. the built-in default.

So a flag always overrides the file for one run. Each on/off setting has a flag for both
directions for exactly that reason: `cost = false` in the file is undone for one run by `--cost`.

## Location

| OS | Path |
| -- | ---- |
| Windows | `%APPDATA%\branch-tally\config.toml` |
| Linux, macOS and others | `$XDG_CONFIG_HOME/branch-tally/config.toml` when `XDG_CONFIG_HOME` is set, non-empty and absolute; otherwise `$HOME/.config/branch-tally/config.toml` |

macOS uses the same rule as Linux: `~/.config/branch-tally/config.toml` unless `XDG_CONFIG_HOME`
is set.

A missing file at the default location is normal: the defaults apply. `--config <path>` on
`report`, `record`, `project` and `config show` reads that file instead, and then the file must
exist (exit 1 if it does not).

Run `branch-tally config show` to see which path applies on your machine.

## Keys

The file is TOML. Every key is optional.

| Key | Type | Default | Flags | Meaning |
| --- | ---- | ------- | ----- | ------- |
| `mode` | string: `"claim"`, `"strict"` or `"timeline"` | `"claim"` | `--mode`, `--strict`, `--timeline` | How calls are assigned to branches. See [Attribution modes](cli.md#attribution-modes). |
| `work-branch` | string (a regex) | not set | `--work-branch` | In timeline mode, which branches count as work branches. Not set means every branch except the default branch and `worktree-agent-*`. Ignored in the other modes. |
| `format` | string: `"md"` or `"json"` | `"md"` | `--format`, `--md`, `--json` | Output format of `report` and `project`. |
| `cost` | boolean | `true` | `--cost`, `--no-cost` | Show the estimated-cost column in Markdown output. |
| `redact` | boolean | `false` | `--redact`, `--no-redact` | Hide identifiers in the report. See [Redaction](cli.md#redaction). |
| `redact-names` | boolean | `true` | `--redact-names`, `--keep-names` | With `redact = true`, replace agent and skill names with generic labels (`true`) or keep the real names (`false`). No effect when `redact` is off. |

An example that reports in timeline mode, counts only `feat/` and `fix/` branches as work, and
redacts by default:

```toml
mode = "timeline"
work-branch = "^(feat|fix)/"
redact = true
```

`record` reads the same file but uses only `mode` and `work-branch`; the output settings don't
apply to it. [`project`](project.md) also reads it; of the output settings it uses only `format`.
A bad config file stops `project` just as it stops `report`.

A `work-branch` in the file while the effective mode is not timeline is ignored, not an error.
`config show` marks it `[inactive: mode is not timeline]`. A `--work-branch` *flag* outside
timeline mode is an error (exit 2).

## Errors

A bad config file stops the command with exit 1 and a message that names the file and, where
there is one, the key. The message never quotes the file's content.

| Problem | Message |
| ------- | ------- |
| Not valid TOML | `config file <path> is not valid TOML: …` |
| Unknown key | ``config file <path>: unknown key `foo` (expected mode, work-branch, format, cost, redact, redact-names)`` |
| Wrong value | ``config file <path>: key `format` must be "md" or "json"`` (and likewise for each key) |
| `work-branch` is not a valid regex | ``config file <path>: key `work-branch` is not a valid regular expression`` |
| `--config` file missing | `config file <path> does not exist` |
| File exists but can't be read | `cannot read config file <path>: …` |

The `work-branch` regex is only checked when it applies, that is in timeline mode.

## `config show`

```sh
branch-tally config show
branch-tally config show --config ./team.toml
```

Prints the path consulted, `found: yes` or `found: no`, and one `key = value (source)` line for
each of `mode`, `work-branch`, `format`, `cost`, `redact` and `redact-names`. The source is
`config file` or `default`. With a config file that sets `mode`, `work-branch` and `redact`:

```text
config file: /home/you/.config/branch-tally/config.toml
found: yes
mode = strict (config file)
work-branch = ^feat/ (config file) [inactive: mode is not timeline]
format = md (default)
cost = true (default)
redact = true (config file)
redact-names = true (default)
```

## `config init`

```sh
branch-tally config init
branch-tally config init --path ./branch-tally.toml
```

Writes a starter file to `--path`, or to the default location when `--path` is omitted. It
creates missing directories and prints `wrote <path>`. It never overwrites an existing file: if the
file exists it fails with `config file <path> already exists; not overwritten` (exit 1).

The file it writes holds every built-in default, with `work-branch` as a commented-out example:

```toml
# branch-tally user config: standing defaults for `report`.
# Precedence: command-line flag, then this file, then the built-in default.

# Attribution mode: "claim", "strict" or "timeline".
mode = "claim"

# With mode = "timeline", a regex naming the work branches (default: every branch except the
# repository's default branch and worktree-agent-*). Ignored in the other modes.
# work-branch = "^(feat|fix)/"

# Output format: "md" or "json".
format = "md"

# Show the estimated-cost column: true or false.
cost = true

# Redact identifiers in the report: session ids become s1, s2, ... and timestamps become dates.
# Costs stay visible; `cost` is independent. true or false.
redact = false

# With redact = true: replace agent and skill names with generic labels (true), or keep the real
# names (false).
redact-names = true
```

The values are written out, not left implicit. If a later release changes a built-in default, a
file made by `config init` keeps the old value until you edit it.
