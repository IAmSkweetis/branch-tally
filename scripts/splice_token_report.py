#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Put a token report into a PR body file before the PR exists.

    uv run scripts/splice_token_report.py BODY.md REPORT.md

BODY.md is the PR body (from .github/PULL_REQUEST_TEMPLATE.md). REPORT.md is the stdout of
`branch-tally report --from-ledger`, which brackets the table with the same two marker lines. The
first start line to the first end line after it in BODY.md is replaced, markers included, and
BODY.md is rewritten. A marker counts only on a line of its own, as `branch-tally report --pr`
treats it. Exits non-zero, changing nothing, when either side lacks the markers (for example when
the ledger had no records for the branch), so a placeholder is never mistaken for a report.
"""

from __future__ import annotations

import sys
from pathlib import Path

START = "<!-- token-report:start -->"
END = "<!-- token-report:end -->"


class SpliceError(Exception):
    pass


def _region(text: str, what: str) -> tuple[int, int]:
    """Line indexes (start, end) of the first start line and the first end line after it."""
    lines = text.splitlines()
    start = next((i for i, ln in enumerate(lines) if ln.strip() == START), None)
    if start is None:
        raise SpliceError(f"{what} has no {START} line")
    end = next((i for i in range(start + 1, len(lines)) if lines[i].strip() == END), None)
    if end is None:
        raise SpliceError(f"{what} has no {END} line after the start marker")
    return start, end


def splice(body: str, report: str) -> str:
    b_start, b_end = _region(body, "the PR body")
    r_start, r_end = _region(report, "the report")
    b_lines = body.splitlines()
    new = report.splitlines()[r_start : r_end + 1]
    out = "\n".join(b_lines[:b_start] + new + b_lines[b_end + 1 :])
    return out + "\n" if body.endswith("\n") else out


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: splice_token_report.py BODY.md REPORT.md", file=sys.stderr)
        return 2
    body_path = Path(argv[1])
    try:
        out = splice(
            body_path.read_text(encoding="utf-8"), Path(argv[2]).read_text(encoding="utf-8")
        )
    except SpliceError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    body_path.write_text(out, encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
