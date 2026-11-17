"""Fail when a page of the built manual lacks the Content-Security-Policy or runs inline script.

The policy is a <meta> in the mdBook theme override (docs/manual/theme/index.hbs), because GitHub
Pages cannot send headers. It only protects if every page carries it, `script-src` stays
`'self'` and no page has an inline <script> or an `on...=` handler (which that policy would block
and so would silently break the page). Reads the built HTML only; needs neither mdbook nor a
browser. Usage: check_manual.py [book_dir]  (default docs/manual/book).
"""

from __future__ import annotations

import sys
from html.parser import HTMLParser
from pathlib import Path

# <script type=...> values that are data blocks, never executed, so CSP does not apply.
DATA_TYPES = {"application/json", "application/ld+json", "importmap", "speculationrules"}


class _Page(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.policies: list[str] = []
        self.inline_scripts = 0
        self.scripts = 0
        self.handlers: set[str] = set()
        self._script_has_src = False
        self._script_is_data = False
        self._in_script = False

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        names = {k: (v or "") for k, v in attrs}
        for k in names:
            if k.startswith("on"):
                self.handlers.add(k)
        if tag == "meta" and names.get("http-equiv", "").lower() == "content-security-policy":
            self.policies.append(names.get("content", ""))
        if tag == "script":
            self.scripts += 1
            self._in_script = True
            self._script_has_src = "src" in names
            self._script_is_data = names.get("type", "").lower() in DATA_TYPES

    def handle_data(self, data: str) -> None:
        if self._in_script and not self._script_has_src and not self._script_is_data:
            if data.strip():
                self.inline_scripts += 1

    def handle_endtag(self, tag: str) -> None:
        if tag == "script":
            self._in_script = False


def _script_src(policy: str) -> list[str] | None:
    for directive in policy.split(";"):
        parts = directive.split()
        if parts and parts[0].lower() == "script-src":
            return parts[1:]
    return None


def check_page(html: str, is_frame: bool = False) -> list[str]:
    """`is_frame`: mdBook's toc.html, the no-JS sidebar iframe, which has no template and so no
    policy. It needs none while it has no script at all."""
    page = _Page()
    page.feed(html)
    problems = []
    if not page.policies and (page.scripts or not is_frame):
        problems.append("no Content-Security-Policy <meta>")
    for policy in page.policies:
        if _script_src(policy) != ["'self'"]:
            problems.append(f"script-src must be exactly 'self', the policy says: {policy!r}")
    if page.inline_scripts:
        problems.append(f"{page.inline_scripts} inline <script> block(s); move them to a file")
    for handler in sorted(page.handlers):
        problems.append(f"inline event handler attribute {handler}=")
    return problems


def check(book: Path) -> list[str]:
    book = Path(book)
    pages = sorted(book.rglob("*.html"))
    if not pages:
        return [f"{book}: no built HTML pages; run `just docs` first"]
    problems = []
    for page in pages:
        text = page.read_text(encoding="utf-8")
        rel = page.relative_to(book).as_posix()
        problems += [f"{rel}: {p}" for p in check_page(text, is_frame=(rel == "toc.html"))]
    return problems


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "docs" / "manual" / "book"
    book = Path(sys.argv[1]) if len(sys.argv) > 1 else default
    problems = check(book)
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    if not problems:
        count = len(list(book.rglob("*.html")))
        print(f"manual ok: {count} pages carry the CSP and no inline script")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
