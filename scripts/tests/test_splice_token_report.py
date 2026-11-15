"""Tests for scripts/splice_token_report.py: the table goes between the markers, or it fails."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import splice_token_report as sp  # noqa: E402

START = "<!-- token-report:start -->"
END = "<!-- token-report:end -->"
BODY = f"## Summary\n\n- why\n\n## Token usage\n\n{START}\n_Not yet recorded._\n{END}\n\ntail\n"
REPORT = f"note: ignored\n{START}\n| Model |\n| `x` |\n{END}\n"


class SpliceTests(unittest.TestCase):
    def test_replaces_placeholder_between_markers(self) -> None:
        out = sp.splice(BODY, REPORT)
        self.assertIn("| `x` |", out)
        self.assertNotIn("_Not yet recorded._", out)
        self.assertTrue(out.startswith("## Summary"))
        self.assertTrue(out.endswith("\ntail\n"))
        self.assertEqual(out.count(START), 1)

    def test_idempotent(self) -> None:
        once = sp.splice(BODY, REPORT)
        self.assertEqual(sp.splice(once, REPORT), once)

    def test_body_without_markers_is_rejected(self) -> None:
        with self.assertRaises(sp.SpliceError):
            sp.splice("## Summary\n", REPORT)

    def test_report_without_markers_is_rejected(self) -> None:
        with self.assertRaises(sp.SpliceError):
            sp.splice(BODY, "No ledger records found\n")

    def test_marker_quoted_in_prose_is_not_a_marker(self) -> None:
        body = f"text {START} inline\n{BODY}"
        out = sp.splice(body, REPORT)
        self.assertTrue(out.startswith(f"text {START} inline\n"))
        self.assertIn("| `x` |", out)

    def test_crlf_body_is_handled(self) -> None:
        body = BODY.replace("\n", "\r\n")
        out = sp.splice(body, REPORT)
        self.assertIn("| `x` |", out)
        self.assertNotIn("_Not yet recorded._", out)
        self.assertTrue(out.startswith("## Summary"))

    def test_only_the_first_marker_pair_is_replaced(self) -> None:
        body = BODY + f"\n{START}\nsecond\n{END}\n"
        out = sp.splice(body, REPORT)
        self.assertIn("| `x` |", out)
        self.assertIn("second", out)
        self.assertEqual(out.count(START), 2)

    def test_no_trailing_newline_is_preserved(self) -> None:
        out = sp.splice(BODY.rstrip("\n"), REPORT)
        self.assertIn("| `x` |", out)
        self.assertFalse(out.endswith("\n"))


if __name__ == "__main__":
    unittest.main()
