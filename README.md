# branch-tally

**What did this pull request cost in AI-agent tokens?**

branch-tally reads an agent harness's local transcripts, assigns each recorded API call to the git
branch the work was for, and publishes a cost table on the PR. That includes work done by subagents
in git worktrees. Claude Code is the first harness supported; others follow.

> **Status: pre-alpha.** Prebuilt binaries ship on the
> [releases page](https://github.com/IAmSkweetis/branch-tally/releases), and this repository's own
> PR token reports are produced by branch-tally. Follow the
> [roadmap](https://github.com/IAmSkweetis/branch-tally/issues/16) for progress.

## Why build branch-tally?

As I've been working on different projects, I like to keep an eye on the costs of a PR and a
project. There are not any good tools for doing this. I had found different plugins that get you
partly there. The different labs do not provide the best metrics or tools for this either.

What I had found are different projects that will give you a running token cost in a terminal. Or a
plugin or extension for different harnesses that are specific to a branch. But nothing really fit
how I think most folks are using AI in their workflows. And there is definitely a gap on a
larger project calculation.

So I had created a [python script](reference/token_report.py) and added some instructions to a skill
to include the usage costs into my PRs.

The goal with branch-tally is to expand on this idea. And honestly, fix some bugs that I've been
putting off, too. We'll see where we get.

## Planned features

- **Per-branch and per-PR totals** of tokens and estimated cost, by model.
- **Worktree subagents rolled up** to the branch that dispatched them.
- **Honest numbers.** Usage that can't be attributed with certainty is counted and reported as
  uncertain, never dropped or guessed. A partial total is shown as a floor ("at least $X").
- **Private by design.** Only usage, model, ids, timestamps, branch and cwd are read. Message
  content never leaves your machine, and a redaction toggle hides identifiers on request.
- **Ships as** a CLI, a Claude Code plugin and skill, and a GitHub Action.

## Install

Linux (x86_64, aarch64) and macOS (Intel, Apple silicon):

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex
```

The binary goes to `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows). The installers never edit
your shell startup files or the Windows user PATH; if the directory is not on `PATH` they print the
line to add. They use only tools a stock system has (curl or wget, `sha256sum` or `shasum`, `tar`;
built-in PowerShell on Windows), and verify the archive against the release's `SHA256SUMS` before
installing anything. A checksum mismatch, or an archive missing from `SHA256SUMS`, is a hard
failure. Provenance attestations are not checked by the installers; to verify one by hand, see
[release.md](docs/references/release.md#verifying-a-download).

| | `install.sh` | `install.ps1` |
| - | ------------ | ------------- |
| Version | `--version vX.Y.Z` or `BRANCH_TALLY_INSTALL_VERSION` | `-Version` or the same variable |
| Directory | `--to <dir>` or `BRANCH_TALLY_INSTALL_DIR` | `-To` or the same variable |
| Mirror | `BRANCH_TALLY_INSTALL_BASE_URL` | the same variable |

A flag overrides its variable. The default version is the latest release. To pin one through a
pipe: `curl -fsSL <url> | sh -s -- --version v0.4.0`, or set the variable.

## Config file

Default values for `report`'s flags can live in a TOML file. Precedence is flag, then config file,
then built-in default. A missing file at the default location is normal.

Location: `%APPDATA%\branch-tally\config.toml` on Windows; elsewhere
`$XDG_CONFIG_HOME/branch-tally/config.toml` (when set, absolute and non-empty), else
`$HOME/.config/branch-tally/config.toml`. `--config <path>` (on `report` and `config show`) reads
that file instead, and it must exist (exit 1 if not).

```toml
mode = "timeline"              # "claim" (default), "strict" or "timeline"
work-branch = "^(feat|fix)/"   # regex; used only when the effective mode is timeline
format = "json"                # "md" (default) or "json"
cost = false                   # true (default) or false; `--cost` undoes it for one run
redact = true                  # false (default) or true; `--no-redact` undoes it for one run
redact-names = false           # true (default) or false; `--redact-names` undoes it for one run
```

All keys are optional. An unknown key, a bad value, invalid TOML or a `work-branch` that is
not a valid regex stops the run with exit 1 and a message naming the file and the key; the message
never quotes the file's content.

The command line has matching options: `--mode claim|strict|timeline`, `--format md|json`,
`--cost` and `--no-cost`, `--redact` and `--no-redact`, `--keep-names` and `--redact-names`.
`--strict`, `--timeline`, `--md` and `--json` remain as shorthands. A shorthand and its long form
cannot be combined, even when they agree (exit 2), and each on/off pair conflicts.

### Redaction

`--redact` (or `redact = true`) hides identifiers in the report, for the PR table and JSON alike:
session ids become `s1`, `s2`, and so on, agent and skill names become generic labels, worktree
names become `worktree-N`, and timestamps are reduced to dates (the table header then reads
`Span (UTC date)`). The output says so: an `identifiers redacted` footnote, and
`"redacted": true` in JSON. Costs stay visible; `--no-cost` is a separate switch. `--keep-names`
(or `redact-names = false`) keeps real agent and skill names while ids, worktrees and times stay
hidden; it has no effect when redaction is off. `branch-tally config show` prints the effective
`redact` and `redact-names` with their source.

```bash
branch-tally config init [--path <path>]
branch-tally config show [--config <path>]
```

`config init` writes a starter file holding the built-in defaults (`mode = "claim"`,
`format = "md"`, `cost = true`, and `work-branch` as a commented-out example) to `--path`, or to the
location above when `--path` is omitted. It creates missing directories, prints the path it wrote,
and never overwrites an existing file (exit 1). The written values are pinned: if a later release
changes a built-in default, a file made by `config init` keeps the old value until you edit it.

`config show` prints the file path, `found: yes|no`, and one `key = value (source)` line each for
`mode`, `work-branch`, `format` and `cost`, where the source is `flag`, `config file` or `default`
(`config show` takes no report flags, so only the last two appear). A work-branch that is set while
the mode is not timeline carries `[inactive: mode is not timeline]`.

## Price table

Estimated costs come from a price table, never from numbers in the code. Every report states the
table's date and where it came from, and a model with no price is named on stderr and makes the
total an "at least" floor.

The table is chosen by the first of these that exists, and that one is used whole, with no
per-model merging:

1. `.tally/prices.toml` in the repository's main checkout (the override).
2. The per-user cache written by `branch-tally prices update`.
3. The table built into the binary.

The cache is `branch-tally/prices.toml` under `%LOCALAPPDATA%` on Windows, `$HOME/Library/Caches`
on macOS, and `$XDG_CACHE_HOME` (default `$HOME/.cache`) elsewhere.

```bash
branch-tally prices update   # refresh the cache from LiteLLM's price list (the only network call)
branch-tally prices show     # which table reports would use: source, date, model count, path
```

`prices update` downloads LiteLLM's `model_prices_and_context_window.json` from GitHub, keeps the
Anthropic chat models, and replaces the cache only if the download and conversion both succeed.
It sends no account or identifying data, only a `branch-tally/<version>` User-Agent. A malformed
override or cache stops the report with a message naming the file; it is never skipped silently.

The override and cache share one TOML format. Its schema, with every key and default, is the
header of [`prices.toml`](crates/branch-tally-core/data/prices.toml), the bundled table. A minimal
override:

```toml
as_of = "2026-11-09"

[models."claude-opus-5"]
input = 5.0   # $ per million tokens
output = 25.0
```

## Links

| | |
| - | - |
| Roadmap | [#16](https://github.com/IAmSkweetis/branch-tally/issues/16) and its `roadmap` sub-issues |
| Project board | [branch-tally](https://github.com/users/IAmSkweetis/projects/2) |
| Contributing | [CONTRIBUTING.md](CONTRIBUTING.md): workspace setup and how a change lands |
| Code of conduct | [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) |
| Security | [SECURITY.md](SECURITY.md): how to report a vulnerability privately |
| Agent and project rules | [AGENTS.md](AGENTS.md) |
| Reference docs | [docs/references/](docs/references/README.md) |
| Issues | [github.com/IAmSkweetis/branch-tally/issues](https://github.com/IAmSkweetis/branch-tally/issues) |

## Licence

[MIT](LICENSE)
