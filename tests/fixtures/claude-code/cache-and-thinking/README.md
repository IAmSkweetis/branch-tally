# cache-and-thinking

Hand-written and synthetic. Relates to #3 and pricing (5m and 1h cache writes price differently).
Usage shapes observed in Claude Code 2.1.x, 2026-11-08. Every record has input 10, output 20,
cache read 30.

| Request | Shape | Expected usage |
| ------- | ----- | -------------- |
| `req_split` | `cache_creation` 5m 25 / 1h 15, flat total 40 | 5m 25, 1h 15, thinking none |
| `req_flat` | only `cache_creation_input_tokens` 40 | 5m 40, 1h 0 |
| `req_think` | `output_tokens_details.thinking_tokens` 7 | output 20, thinking Some(7) |
| `req_zero` | thinking_tokens 0 | thinking Some(0), distinct from none |

A correct read reports `files_read` 1 and `records_emitted` 4, every skip counter 0. Thinking is
never added to output.
