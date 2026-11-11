# Getting Started

This page takes you from nothing to a cost table for a branch. You need a git repository where you
have worked with Claude Code, on the same machine that ran the sessions.

## 1. Install

Linux (x86_64, aarch64) and macOS (Intel, Apple silicon):

```sh
curl -fsSL https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/IAmSkweetis/branch-tally/releases/latest/download/install.ps1 | iex
```

Both put the binary in `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows) and print the line
to add if that directory is not on your `PATH`. Check it worked:

```sh
branch-tally --version
```

[Installation](installation/README.md) covers the options, verifying a download by hand and
building from source.

## 2. Run your first report

From inside the repository, with the branch you want checked out:

```sh
branch-tally report
```

That reports on the current branch. To report on another branch, name it:

```sh
branch-tally report feat/my-change
```

branch-tally looks for transcripts under `~/.claude/projects/` (`%USERPROFILE%\.claude\projects`
on Windows) for this repository's main checkout and each of its git worktrees. It only reads them.

If it prints `No transcript records found for branch '…'`, the likely causes are:

- the sessions ran on another machine, or under a different working directory;
- the transcripts are older than Claude Code's retention period (`cleanupPeriodDays`, 30 days by
  default) and were deleted;
- the branch name is misspelled.

To keep usage beyond the retention period, record it in the [ledger](ledger.md).

## 3. Read the output

This output was produced by running `branch-tally report main` against one of branch-tally's
scrubbed test fixtures, so the session id, dates and token counts are synthetic:

```text
warning: no price for claude-mystery-9 in the price table (as of 2026-11-09); the total is a floor
<!-- token-report:start -->
| Model | Calls | Input | Cache write | Cache read | Output | (thinking) | Est. cost |
|---|--:|--:|--:|--:|--:|--:|--:|
| `claude-opus-5[1m]` | 2 | 7,654,324 | 930,894 | 98,765,443 | 123,464 | 0 | $96.76 |
| `claude-sonnet-4-6` | 2 | 1,235,344 | 269,356 | 4,569,002 | 89,456 | 0 | $7.51 |
| `claude-fable-5-1` | 2 | 112,344 | 20,428 | 3,543,209 | 6,789 | 0 | $2.66 |
| `claude-mystery-9` | 1 | 11,111 | 499 | 3,333 | 2,222 | 0 | — |
| **Total** | **7** | **9,013,123** | **1,221,177** | **106,880,987** | **221,931** | **0** | **at least $106.93** |

| Session | Where | Agents | Span (UTC) | Calls | Tokens |
|---|---|---|---|--:|--:|
| `s1` | main checkout | — | 01-01 00:00 → 01-01 00:00 | 7 | 117.3M |

<sub>Branch `main` · 1 session(s), 0 in a worktree · 0 of 7 calls from subagents · prices as of 2026-11-09 (bundled table) · 1 model(s) unpriced, total is a floor · est. cost at list API rates, not amounts billed</sub>

<details><summary>Breakdown by skill / agent</summary>

| Attribution | Calls | Tokens |
|---|--:|--:|
| main | 7 | 117,337,218 |

</details>
<!-- token-report:end -->
```

The first line went to stderr; everything else is stdout. Piece by piece:

- **The markers.** `<!-- token-report:start -->` and `<!-- token-report:end -->` wrap the report,
  so `report --pr` can find and replace it in a pull request body. In a PR they are invisible.
- **The model table.** One row per model: API calls, then the token counts the API reported.
  "Cache write" adds the 5-minute and 1-hour cache writes, which are priced differently; the JSON
  output keeps them apart. "(thinking)" is thinking tokens, shown separately where the transcript
  records them. "Est. cost" is the estimate at list API rates.
- **A floor.** `claude-mystery-9` is not in the price table, so its cost is `—`, stderr names it,
  and the total reads "at least $106.93". The tokens are still counted.
- **The session table.** One row per Claude Code session that contributed: where it ran (the main
  checkout or a worktree), the named agents it used, the UTC time span, its calls and its tokens in
  millions.
- **The footnote.** The branch, how many sessions ran in a worktree, how many calls came from
  subagents, which attribution heuristics were used (with their call and token counts), and the
  price table's date and source.
- **Breakdown by skill / agent.** The same tokens split by the skill or named agent that made the
  calls. `main` is the session's own top-level work.

When subagents did part of the work, a **Subagent usage** section follows, with one row per
subagent, the workspace it ran in and its cost, plus a line saying how many subagent calls had no
dispatch link. When some calls could not be assigned to a branch with certainty, a bold line says
"Token total is at least …" and how many tokens were ambiguous.

## Next steps

- Put the table on your pull request: `branch-tally report --pr --dry-run` shows the new PR body,
  and `branch-tally report --pr` writes it. You need the [GitHub CLI](https://cli.github.com/)
  installed and signed in, and an open PR for the branch.
- Get machine-readable output with `--json`.
- Choose how calls are assigned to branches with `--mode` (see
  [Attribution modes](cli.md#attribution-modes)).
- Save your preferred defaults in a [config file](config.md).
- Keep the numbers after transcripts are purged with the [ledger](ledger.md).
