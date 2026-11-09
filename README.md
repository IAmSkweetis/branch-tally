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
