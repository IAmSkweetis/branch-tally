"""Tests for scripts/check_manual.py: a built page with an inline script or no CSP is rejected."""

from __future__ import annotations

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import check_manual as cm  # noqa: E402

CSP = ("<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'; "
       "script-src 'self'; object-src 'none'; base-uri 'self'\">")
GOOD = f"<html><head>{CSP}<script src=\"boot.js\"></script></head><body><p>x</p></body></html>"


def make_book(pages: dict[str, str]) -> Path:
    root = Path(tempfile.mkdtemp())
    for name, html in pages.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(html, encoding="utf-8")
    return root


class CheckManual(unittest.TestCase):
    def check(self, pages: dict[str, str]) -> list[str]:
        root = make_book(pages)
        self.addCleanup(shutil.rmtree, root, True)
        return cm.check(root)

    def test_good_page_is_clean(self):
        self.assertEqual(self.check({"index.html": GOOD}), [])

    def test_inline_script_rejected(self):
        bad = GOOD.replace("<p>x</p>", "<script>alert(1)</script>")
        problems = self.check({"index.html": GOOD, "a/b.html": bad})
        self.assertEqual(len(problems), 1)
        self.assertIn("b.html", problems[0])
        self.assertIn("inline", problems[0])

    def test_inline_script_with_attributes_rejected(self):
        bad = GOOD.replace("<p>x</p>", "<script type=\"module\">x()</script>")
        self.assertTrue(self.check({"index.html": bad}))

    def test_data_block_is_not_a_script(self):
        ok = GOOD.replace("<p>x</p>", "<script type=\"application/json\">{}</script>")
        self.assertEqual(self.check({"index.html": ok}), [])

    def test_missing_csp_rejected(self):
        bad = GOOD.replace(CSP, "")
        self.assertTrue(any("Content-Security-Policy" in p for p in self.check({"i.html": bad})))

    def test_unsafe_inline_script_src_rejected(self):
        bad = GOOD.replace("script-src 'self'", "script-src 'self' 'unsafe-inline'")
        self.assertTrue(any("script-src" in p for p in self.check({"i.html": bad})))

    def test_script_src_must_be_self_only(self):
        bad = GOOD.replace("script-src 'self'", "script-src 'self' https://cdn.example")
        self.assertTrue(self.check({"i.html": bad}))

    def test_event_handler_attribute_rejected(self):
        bad = GOOD.replace("<p>x</p>", "<p onclick=\"x()\">x</p>")
        self.assertTrue(any("onclick" in p for p in self.check({"i.html": bad})))

    def test_empty_book_rejected(self):
        self.assertTrue(self.check({"readme.txt": "x"}))

    def test_script_free_sidebar_frame_needs_no_csp(self):
        # mdBook's toc.html is the no-JS sidebar iframe: it has no scripts, so no policy to carry.
        frame = "<html><body><ol><li>a</li></ol></body></html>"
        self.assertEqual(self.check({"index.html": GOOD, "toc.html": frame}), [])

    def test_sidebar_frame_with_inline_script_rejected(self):
        frame = "<html><body><script>x()</script></body></html>"
        self.assertTrue(self.check({"index.html": GOOD, "toc.html": frame}))

    def test_print_page_is_checked_too(self):
        bad = GOOD.replace(CSP, "")
        self.assertTrue(self.check({"print.html": bad}))


if __name__ == "__main__":
    unittest.main()
