# Integration

branch-tally works without any integration: `branch-tally report` reads the transcripts an agent
harness has already written on your machine. An integration wires branch-tally into the harness
itself, so the harness's hooks run branch-tally while you work. They note which branch each session
was on, as [hook captures](../captures.md), and fill the [ledger](../ledger.md) before old
transcripts are purged.

| Harness | Page | Sets up |
| ------- | ---- | ------- |
| Claude Code | [Claude Code](claude-code.md) | Two optional plugins: `branch-tally-capture` for hook captures and `branch-tally-record` for the ledger. |

The files an integration writes are the same whichever harness wrote them. Each line of
`.tally/captures.jsonl` and `.tally/ledger.jsonl` names its harness in a `harness` field, and
reports read both files the same way.

## Publishing the report

The integrations record usage. They don't publish it. To put the table on a pull request, run
`branch-tally report --pr`, or `branch-tally report --pr --from-ledger` to publish from a committed
ledger. See [Updating a pull request](../cli.md#updating-a-pull-request) in the CLI reference. A
Claude Code skill that records and publishes a PR's usage
([#11](https://github.com/IAmSkweetis/branch-tally/issues/11)) and a GitHub Action
([#12](https://github.com/IAmSkweetis/branch-tally/issues/12)) are planned and not available yet.
