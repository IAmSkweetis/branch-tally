# branch-tally Claude Code plugins

Optional hooks that keep branch-tally's numbers from being lost when Claude Code purges old
transcripts. They are separate plugins so you install only the ones you want.

| Plugin | Hooks | Runs |
| ------ | ----- | ---- |
| `branch-tally-capture` | `SessionStart`, `SubagentStop`, `Stop` | `branch-tally capture` |
| `branch-tally-record` | `SessionEnd` | `branch-tally record` |

## Install

Both need the `branch-tally` binary on your `PATH`. Without it the hooks do nothing.

```text
/plugin marketplace add IAmSkweetis/branch-tally
/plugin install branch-tally-capture@branch-tally
/plugin install branch-tally-record@branch-tally
```

Install either, both or neither. To opt out later, run `/plugin disable <name>` or
`/plugin uninstall <name>`.

## What the hooks do and do not do

- They read only the hook payload Claude Code sends on stdin, and run branch-tally in the session's
  working directory. Message content is never read or written (see the transcript privacy rule).
- Run the session from inside the repository. In any other directory the commands find nothing to
  record.
- Every command line ends in `; exit 0`, which both Bash and PowerShell accept. A missing binary, a
  directory that is not a repository or any other failure can never exit with code 2, which would
  block `Stop` and `SubagentStop`. Errors may still print to stderr.
- A total built from captured data is a floor when attribution was uncertain, as with `report`.

## SessionEnd time limit

Claude Code gives `SessionEnd` hooks a default budget of 1.5 seconds, and that is all a plugin can
rely on. A per-hook `timeout` raises the budget only when it is set in your own settings files:
"Timeouts set on plugin-provided hooks don't raise the budget." The record plugin therefore sets no
`timeout`. Source: <https://code.claude.com/docs/en/hooks> (SessionEnd section), read 2026-11-11.

`branch-tally record` reads the repository's transcripts, so a large history can take longer than
1.5 seconds. To give it more time, either:

- set the environment variable in milliseconds before starting Claude Code, for example
  `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS=10000 claude`. The value also becomes the timeout of
  hooks that have none. Before Claude Code v2.1.268 it raised only the overall budget.
- or add the hook to your own settings file with a `timeout` (up to 60 seconds).

If `record` is killed, nothing is lost for good. It appends only entries whose key the ledger does
not already hold, so the next `SessionEnd`, or a manual `branch-tally record`, catches up. The
ledger write is one `write_all` on an append-mode file. A kill during that call could in principle
leave a truncated last line. The reader counts such a line as malformed instead of failing, the
next append starts on a fresh line, and the truncated entry is written again whole because its key
was never recorded. Until then the ledger total is a floor.

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
