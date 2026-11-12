# branch-tally Claude Code plugins

Optional hooks that keep branch-tally's numbers from being lost when Claude Code purges old
transcripts. They are separate plugins so you install only the ones you want.

| Plugin | Hooks | Runs |
| ------ | ----- | ---- |
| `branch-tally-capture` | `SessionStart`, `SubagentStop`, `Stop` | `branch-tally capture` |
| `branch-tally-record` | `SessionEnd` | `branch-tally record` |

## Install

Both need the `branch-tally` binary on your `PATH`. Without it the hooks do nothing. Installing a
plugin does not turn recording on for a repository: see [Opt a repository in](#opt-a-repository-in).

```text
/plugin marketplace add IAmSkweetis/branch-tally
/plugin install branch-tally-capture@branch-tally
/plugin install branch-tally-record@branch-tally
```

Install either, both or neither. To opt out later, run `/plugin disable <name>` or
`/plugin uninstall <name>`.

## Opt a repository in

The hooks never create `.tally/`. They write only when `.tally/` already exists as a real
directory, and otherwise exit quietly and create nothing. Opt a repository in once, from its root:

```text
mkdir .tally
touch .tally/.gitkeep
```

Commit the directory (the `.gitkeep`, or an existing ledger) so it is there in every clone.

- `branch-tally capture` writes `.tally/captures.jsonl` in the main checkout's `.tally/`.
- `branch-tally record` writes `.tally/ledger.jsonl` in the git top level's `.tally/`.
- A symlink at `.tally`, `.tally/captures.jsonl`, `.tally/ledger.jsonl` or `.tally/prices.toml`,
  or any file there that is not a regular file, is refused on read and on write. Nothing is
  followed or created through a link. Replace the link with a real directory or file.
- An explicit `--ledger <path>` is exempt from both rules.

## What the hooks do and do not do

- They read only the hook payload Claude Code sends on stdin, and run branch-tally in the session's
  working directory. Message content is never read or written (see the transcript privacy rule).
- Run the session from inside the repository. In any other directory the commands find nothing to
  record.
- Every command line ends in `; exit 0`, which both Bash and PowerShell accept. A missing binary, a
  directory that is not a repository or any other failure can never exit with code 2, which would
  block `Stop` and `SubagentStop`. Errors may still print to stderr.
- Calls placed using captured data are not marked as a floor. The report's footnote lists "captured
  hook data" among the heuristics used, with its call and token counts (`heuristics.captured` in
  JSON, shown only when used). Calls that still cannot be attributed with certainty are counted
  and make the total "at least $X", exactly as without captures.

## SessionEnd time limit

Claude Code gives `SessionEnd` hooks a default budget of 1.5 seconds, and that is all a plugin can
rely on. A per-hook `timeout` raises the budget only when it is set in your own settings files:
"Timeouts set on plugin-provided hooks don't raise the budget." The record plugin therefore sets no
`timeout`. Source: <https://code.claude.com/docs/en/hooks> (SessionEnd section), read 2026-11-11.

`branch-tally record` reads the repository's transcripts, so on a repository with a lot of history
it will usually be cut off at 1.5 seconds unless you raise the limit. The recommended way is to set
`CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`. To give it more time, either:

- set the environment variable in milliseconds before starting Claude Code, for example
  `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS=10000 claude` (recommended). The value also becomes the
  timeout of hooks that have none. Before Claude Code v2.1.268 it raised only the overall budget.
- or add the hook to your own settings file with a `timeout` (up to 60 seconds).

If `record` is killed, nothing is lost for good. It appends only entries whose key the ledger does
not already hold, so the next `SessionEnd`, or a manual `branch-tally record`, catches up. The
ledger write is one `write_all` on an append-mode file. A kill during that call could in principle
leave a truncated last line. The reader counts such a line as malformed instead of failing, the
next append starts on a fresh line, and the truncated entry is written again whole because its key
was never recorded. Until then reports print `note: skipped N ledger line(s)` on stderr. The
report is not labelled "at least": that label comes only from uncertain attribution or unpriced
models, so a ledger missing an entry can understate the total without saying so in the table.

## Which shell runs the hooks

Hook commands are shell-form. Per the same page, they run through `sh -c` on macOS and Linux, Git
Bash on Windows, or PowerShell on Windows when Git Bash is not installed. `; exit 0` is valid in
all of them. A hook's `shell` field (`"bash"` or `"powershell"`) can choose explicitly.

## Why two plugins

Claude Code can disable a whole plugin but not one hook inside it, and shell-form hooks cannot read
`userConfig` values portably across Bash and PowerShell. Separate plugins are the clean opt-in.

## Versions

The manifests set no `version`, so Claude Code treats each commit as a new version. The project's
release version lives only in the git tag ([release.md](../docs/references/release.md#versioning)).

## Checks

`plugin/tests/check.sh [binary]` validates both manifests and the plugins, checks that every hook
command ends in `; exit 0`, and that each `branch-tally <subcommand>` it names exists in the binary.
