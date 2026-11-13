"""Tests for scripts/check_pins.py: a drifted mise.toml or vendored asset must be rejected."""

from __future__ import annotations

import hashlib
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import check_pins as cp  # noqa: E402

REPO = Path(__file__).resolve().parents[2]

JUSTFILE = '''rumdl_version := "0.3.2"
zizmor_version := "1.31.1"
gitleaks_version := "8.30.1"
cliff_version := "2.13.1"
mdbook_version := "0.5.4"
mdbook_mermaid_version := "0.17.1"
'''
CI = """jobs:
  a:
    steps:
      - uses: taiki-e/install-action@abc # v2
        with:
          tool: just@1.59.0,cargo-nextest@0.9.151
      - uses: taiki-e/install-action@abc # v2
        with:
          tool: cargo-deny@0.20.2
"""
TOOLCHAIN = '[toolchain]\nchannel = "1.99.1"\n'
MISE = """[tools]
rust = "1.99.1"
just = "1.59.0"
"aqua:rust-lang/mdBook" = "0.5.4"
"github:badboy/mdbook-mermaid" = "0.17.1"
"cargo:cargo-nextest" = "0.9.151"
cargo-deny = "0.20.2"
gitleaks = "8.30.1"
git-cliff = "2.13.1"
rumdl = "0.3.2"
zizmor = "1.31.1"
minisign = "0.12"
"""
MINISIGN_SCRIPT = """#!/bin/sh
set -eu

version=0.12
sha256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73
dest=${1:-/usr/local/bin}
"""
ASSET = b"console.log('pan');\n"


def make_repo(mise: str = MISE, justfile: str = JUSTFILE, ci: str = CI, asset: bytes = ASSET,
              sha: str | None = None, minisign: str | None = MINISIGN_SCRIPT) -> Path:
    root = Path(tempfile.mkdtemp())
    (root / ".github" / "workflows").mkdir(parents=True)
    (root / "docs" / "manual").mkdir(parents=True)
    (root / "justfile").write_text(justfile, encoding="utf-8")
    (root / ".github" / "workflows" / "ci.yml").write_text(ci, encoding="utf-8")
    (root / "scripts").mkdir()
    if minisign is not None:
        (root / "scripts" / "ci-install-minisign.sh").write_text(minisign, encoding="utf-8")
    (root / "rust-toolchain.toml").write_text(TOOLCHAIN, encoding="utf-8")
    (root / "mise.toml").write_text(mise, encoding="utf-8")
    (root / "docs" / "manual" / "svg-pan-zoom.min.js").write_bytes(asset)
    digest = sha or hashlib.sha256(asset).hexdigest()
    (root / "docs" / "manual" / "svg-pan-zoom.min.js.sha256").write_text(
        f"{digest}  svg-pan-zoom.min.js\n", encoding="utf-8")
    return root


class CheckPins(unittest.TestCase):
    def check(self, **kw) -> list[str]:
        root = make_repo(**kw)
        self.addCleanup(shutil.rmtree, root, True)
        return cp.check(root)

    def test_agreeing_repo_is_clean(self):
        self.assertEqual(self.check(), [])

    def test_drifted_just_version_rejected(self):
        problems = self.check(mise=MISE.replace('just = "1.59.0"', 'just = "1.58.0"'))
        self.assertEqual(len(problems), 1)
        self.assertIn("just", problems[0])

    def test_drifted_justfile_pin_rejected(self):
        problems = self.check(justfile=JUSTFILE.replace("8.30.1", "8.31.0"))
        self.assertTrue(any("gitleaks" in p for p in problems))

    def test_drifted_rust_rejected(self):
        problems = self.check(mise=MISE.replace('rust = "1.99.1"', 'rust = "1.98.0"'))
        self.assertTrue(any("rust" in p for p in problems))

    def test_missing_tool_rejected(self):
        problems = self.check(mise=MISE.replace('cargo-deny = "0.20.2"\n', ""))
        self.assertTrue(any("cargo-deny" in p and "missing" in p for p in problems))

    def test_unpinned_extra_tool_rejected(self):
        problems = self.check(mise=MISE + 'uv = "0.5.0"\n')
        self.assertTrue(any("uv" in p for p in problems))

    def test_table_form_version_is_read(self):
        mise = MISE.replace('rust = "1.99.1"', 'rust = { version = "1.99.1", profile = "minimal" }')
        self.assertEqual(self.check(mise=mise), [])

    def test_workflows_disagreeing_with_each_other_rejected(self):
        ci = CI + "      - uses: x\n        with:\n          tool: just@1.60.0\n"
        problems = self.check(ci=ci)
        self.assertTrue(any("just" in p for p in problems))

    def test_tampered_vendored_asset_rejected(self):
        problems = self.check(sha="0" * 64)
        self.assertTrue(any("svg-pan-zoom" in p for p in problems))

    def test_missing_mise_toml_rejected(self):
        root = make_repo()
        self.addCleanup(shutil.rmtree, root, True)
        (root / "mise.toml").unlink()
        self.assertTrue(cp.check(root))

    def test_drifted_minisign_version_rejected(self):
        problems = self.check(mise=MISE.replace('minisign = "0.12"', 'minisign = "0.11"'))
        self.assertTrue(any("minisign" in p and "0.12" in p for p in problems))

    def test_minisign_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('minisign = "0.12"\n', ""))
        self.assertTrue(any("minisign" in p and "missing" in p for p in problems))

    def test_minisign_bumped_in_script_only_rejected(self):
        problems = self.check(minisign=MINISIGN_SCRIPT.replace("version=0.12", "version=0.13"))
        self.assertTrue(any("minisign" in p for p in problems))

    def test_minisign_malformed_sha_rejected(self):
        bad = MINISIGN_SCRIPT.replace("sha256=9a599b", "sha256=zz99b")
        problems = self.check(minisign=bad)
        self.assertTrue(any("minisign" in p and "sha256" in p for p in problems))

    def test_minisign_pin_missing_from_script_rejected(self):
        problems = self.check(minisign="#!/bin/sh\necho nothing pinned\n")
        self.assertTrue(any("minisign" in p for p in problems))

    def test_minisign_script_missing_rejected(self):
        problems = self.check(minisign=None)
        self.assertTrue(any("minisign" in p for p in problems))

    def test_real_repository_agrees(self):
        self.assertEqual(cp.check(REPO), [])


if __name__ == "__main__":
    unittest.main()
