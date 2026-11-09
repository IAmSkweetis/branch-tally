# branch-tally

**What did this pull request cost in AI-agent tokens?**

branch-tally reads an agent harness's local transcripts, assigns each recorded API call to the git
branch the work was for, and publishes a cost table on the PR. That includes work done by subagents
in git worktrees. Claude Code is the first harness supported; others follow.

> **Status: pre-alpha.** Prebuilt binaries ship on the
> [releases page](https://github.com/IAmSkweetis/branch-tally/releases), and this repository's own
> PR token reports are produced by branch-tally. Follow the
> [roadmap](https://github.com/IAmSkweetis/branch-tally/issues/16) for progress.

## Planned features

- **Per-branch and per-PR totals** of tokens and estimated cost, by model.
- **Worktree subagents rolled up** to the branch that dispatched them.
- **Honest numbers.** Usage that can't be attributed with certainty is counted and reported as
  uncertain, never dropped or guessed. A partial total is shown as a floor ("at least $X").
- **Private by design.** Only usage, model, ids, timestamps, branch and cwd are read. Message
  content never leaves your machine, and a redaction toggle hides identifiers on request.
- **Ships as** a CLI, a Claude Code plugin and skill, and a GitHub Action.

## Links

| | |
| - | - |
| Roadmap | [#16](https://github.com/IAmSkweetis/branch-tally/issues/16) and its `roadmap` sub-issues |
| Project board | [branch-tally](https://github.com/users/IAmSkweetis/projects/2) |
| Contributing | [CONTRIBUTING.md](CONTRIBUTING.md): workspace setup and how a change lands |
| Agent and project rules | [AGENTS.md](AGENTS.md) |
| Reference docs | [docs/references/](docs/references/README.md) |
| Issues | [github.com/IAmSkweetis/branch-tally/issues](https://github.com/IAmSkweetis/branch-tally/issues) |

## Licence

[MIT](LICENSE)
