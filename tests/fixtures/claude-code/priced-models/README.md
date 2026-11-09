# priced-models

Hand-written and synthetic. Relates to #4 (cost parity with `reference/token_report.py`). Usage
shapes observed in Claude Code 2.1.x, 2026-11-08; token counts are invented. Every other scenario
uses the unpriced `claude-test-1`, so this is the one that compares cost cells, the Total cost,
`totalCostUSD` and per-model `costUSD` against the reference. Content fields are `PLACEHOLDER`.

One session, all on `main`, 7 calls over 4 models (first-seen order is sonnet, fable, opus,
mystery):

| Model id in the transcript | Prices as | Why it is here |
| -------------------------- | --------- | -------------- |
| `claude-sonnet-4-6` | exact match, 3/15 | plain priced model |
| `claude-fable-5-1` | exact match, 10/50 | cache-read override 0.25 instead of 0.1x |
| `claude-opus-5[1m]` | prefix match on `claude-opus-5`, 5/25 | suffixed id |
| `claude-mystery-9` | no price | `—` / null cost beside priced models |

Every call has non-zero 5m and 1h cache writes (one sonnet call has 1h 0) and large cache reads,
so each per-model cost has many significant digits. Models are ordered by cost in the output,
which differs from first-seen order, so the markdown total and the JSON total sum in different
orders.

A correct report prices three models and shows the unknown model as unpriced rather than zero.
Goldens come from `just goldens priced-models`; never edit them by hand.
