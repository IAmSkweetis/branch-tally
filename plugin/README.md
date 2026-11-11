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

## Why two plugins

Claude Code can disable a whole plugin but not one hook inside it, and shell-form hooks cannot read
`userConfig` values portably across Bash and PowerShell. Separate plugins are the clean opt-in.

## Versions

The manifests set no `version`, so Claude Code treats each commit as a new version. The project's
release version lives only in the git tag ([release.md](../docs/references/release.md#versioning)).

## Checks

`plugin/tests/check.sh [binary]` validates both manifests and the plugins, checks that every hook
command ends in `; exit 0`, and that each `branch-tally <subcommand>` it names exists in the binary.
