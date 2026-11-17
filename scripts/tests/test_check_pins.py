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
jq_version := "1.8.1"
gh_version := "2.104.0"
'''
SETUP_UV = """      - uses: astral-sh/setup-uv@abc # v10
        with:
          enable-cache: false
          version: "0.13.8"
"""
CI = """jobs:
  a:
    steps:
      - uses: taiki-e/install-action@abc # v2
        with:
          tool: just@1.59.0,cargo-nextest@0.9.151
      - uses: taiki-e/install-action@abc # v2
        with:
          tool: cargo-deny@0.20.2
""" + SETUP_UV
SETTINGS = """{
  "permissions": {"allow": ["Bash(uvx rumdl@0.3.2 check *)", "Bash(just ci)"]},
  "hooks": {"PostToolUse": [{"hooks": [{"type": "command",
    "command": "f=$(jq -r .x); case $f in *.md) uvx rumdl@0.3.2 check $f;; esac"}]}]}
}
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
uv = "0.13.8"
jq = "1.8.1"
gh = "2.104.0"
python = "3.15.2"
zizmor = "1.31.1"
minisign = "0.12"
powershell = "7.7.0"
"""
MINISIGN_SCRIPT = """#!/bin/sh
set -eu

version=0.12
sha256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73
dest=${1:-/usr/local/bin}
"""
PWSH_SCRIPT = """#!/bin/sh
set -eu

version=7.7.0
sha256_linux_x64=c89d9c682d2c403dfe726f3df948e359675235a5bd6fab535776de69729ca0cb
sha256_linux_arm64=e59c89776eaa59790d44b3edd14277435225ab08b6e7603763101b89de83d935
"""
ASSET = b"console.log('pan');\n"


MERMAID = b"// stand-in for the vendored mermaid bundle\n"


def make_repo(mise: str = MISE, justfile: str = JUSTFILE, ci: str = CI, asset: bytes = ASSET,
              sha: str | None = None, settings: str | None = SETTINGS, python: str | None = "3.15.2\n", minisign: str | None = MINISIGN_SCRIPT,
              pwsh: str | None = PWSH_SCRIPT, mermaid_sha: str | None = None) -> Path:
    root = Path(tempfile.mkdtemp())
    (root / ".github" / "workflows").mkdir(parents=True)
    (root / "docs" / "manual").mkdir(parents=True)
    (root / "justfile").write_text(justfile, encoding="utf-8")
    (root / ".github" / "workflows" / "ci.yml").write_text(ci, encoding="utf-8")
    (root / "scripts").mkdir()
    if minisign is not None:
        (root / "scripts" / "ci-install-minisign.sh").write_text(minisign, encoding="utf-8")
    if pwsh is not None:
        (root / "scripts" / "ci-install-pwsh.sh").write_text(pwsh, encoding="utf-8")
    (root / "rust-toolchain.toml").write_text(TOOLCHAIN, encoding="utf-8")
    if python is not None:
        (root / ".python-version").write_text(python, encoding="utf-8")
    if settings is not None:
        (root / ".claude").mkdir()
        (root / ".claude" / "settings.json").write_text(settings, encoding="utf-8")
    (root / "mise.toml").write_text(mise, encoding="utf-8")
    (root / "docs" / "manual" / "svg-pan-zoom.min.js").write_bytes(asset)
    digest = sha or hashlib.sha256(asset).hexdigest()
    (root / "docs" / "manual" / "svg-pan-zoom.min.js.sha256").write_text(
        f"{digest}  svg-pan-zoom.min.js\n", encoding="utf-8")
    (root / "docs" / "manual" / "mermaid.min.js").write_bytes(MERMAID)
    (root / "docs" / "manual" / "mermaid.min.js.sha256").write_text(
        f"{mermaid_sha or hashlib.sha256(MERMAID).hexdigest()}  mermaid.min.js\n",
        encoding="utf-8")
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
        problems = self.check(mise=MISE + 'ripgrep = "14.0.0"\n')
        self.assertTrue(any("ripgrep" in p for p in problems))

    def test_settings_rumdl_drift_in_hook_rejected(self):
        bad = SETTINGS.replace("*.md) uvx rumdl@0.3.2", "*.md) uvx rumdl@0.3.1")
        problems = self.check(settings=bad)
        self.assertTrue(any("rumdl" in p and "0.3.1" in p for p in problems))

    def test_settings_rumdl_drift_in_permission_rejected(self):
        bad = SETTINGS.replace("Bash(uvx rumdl@0.3.2", "Bash(uvx rumdl@0.2.0")
        problems = self.check(settings=bad)
        self.assertTrue(any("rumdl" in p and "0.2.0" in p for p in problems))

    def test_settings_unpinned_uvx_rejected(self):
        bad = SETTINGS.replace("uvx rumdl@0.3.2 check $f", "uvx rumdl check $f")
        problems = self.check(settings=bad)
        self.assertTrue(any("unpinned" in p and "rumdl" in p for p in problems))

    def test_settings_unpinned_other_uvx_rejected(self):
        bad = SETTINGS.replace("Bash(just ci)", "Bash(uvx ruff check *)")
        problems = self.check(settings=bad)
        self.assertTrue(any("unpinned" in p and "ruff" in p for p in problems))

    def test_settings_missing_rejected(self):
        problems = self.check(settings=None)
        self.assertTrue(any("settings.json" in p for p in problems))

    def test_setup_uv_without_version_rejected(self):
        ci = CI.replace('          version: "0.13.8"\n', "")
        problems = self.check(ci=ci)
        self.assertTrue(any("setup-uv" in p and "version" in p for p in problems))

    def test_named_setup_uv_step_is_read(self):
        named = SETUP_UV.replace("      - uses: astral-sh/setup-uv",
                                 "      - name: Install uv\n        uses: astral-sh/setup-uv")
        self.assertEqual(self.check(ci=CI + named), [])

    def test_named_setup_uv_step_without_version_rejected(self):
        named = SETUP_UV.replace("      - uses: astral-sh/setup-uv",
                                 "      - name: Install uv\n        uses: astral-sh/setup-uv")
        named = named.replace('          version: "0.13.8"\n', "")
        problems = self.check(ci=CI + named + "      - run: echo next\n")
        self.assertTrue(any("setup-uv" in p and "version" in p for p in problems))

    def test_setup_uv_version_before_uses_is_read(self):
        reordered = ("      - name: Install uv\n"
                     "        with:\n"
                     '          version: "0.13.8"\n'
                     "        uses: astral-sh/setup-uv@abc # v10\n")
        self.assertEqual(self.check(ci=CI + reordered), [])

    def test_settings_floating_uvx_version_rejected(self):
        bad = SETTINGS.replace("Bash(just ci)", "Bash(uvx ruff@latest check *)")
        problems = self.check(settings=bad)
        self.assertTrue(any("ruff" in p and "latest" in p for p in problems))

    def test_settings_uvx_after_escaped_newline_rejected(self):
        # The JSON text holds `\n`, so the raw text has an `n` directly before `uvx`.
        bad = SETTINGS.replace('"type": "command",',
                               '"type": "command", "command": "echo ready\\nuvx ruff check ."}, '
                               '{"type": "command",')
        self.assertIn("ready\\nuvx ruff", bad)
        problems = self.check(settings=bad)
        self.assertTrue(any("unpinned" in p and "ruff" in p for p in problems), problems)

    def test_settings_invalid_json_rejected(self):
        problems = self.check(settings=SETTINGS.replace('"hooks": {', '"hooks" {', 1))
        self.assertTrue(any("not valid JSON" in p for p in problems), problems)

    def test_settings_exact_command_permission_is_accepted(self):
        good = SETTINGS.replace('"Bash(just ci)"', '"Bash(just ci)",\n      "Bash(uvx ruff@0.11.0)"')
        self.assertIn("Bash(uvx ruff@0.11.0)", good)
        self.assertEqual(self.check(settings=good), [])

    def test_setup_uv_steps_disagreeing_rejected(self):
        ci = CI + SETUP_UV.replace("0.13.8", "0.13.7")
        problems = self.check(ci=ci)
        self.assertTrue(any("uv" in p and "disagree" in p for p in problems))

    def test_mise_uv_drift_rejected(self):
        problems = self.check(mise=MISE.replace('uv = "0.13.8"', 'uv = "0.12.0"'))
        self.assertTrue(any("uv" in p and "0.13.8" in p for p in problems))

    def test_uv_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('uv = "0.13.8"\n', ""))
        self.assertTrue(any("uv" in p and "missing" in p for p in problems))

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

    def test_tampered_mermaid_bundle_rejected(self):
        problems = self.check(mermaid_sha="0" * 64)
        self.assertTrue(any("mermaid.min.js" in p for p in problems))

    def test_missing_mermaid_record_rejected(self):
        root = make_repo()
        self.addCleanup(shutil.rmtree, root, True)
        (root / "docs" / "manual" / "mermaid.min.js.sha256").unlink()
        self.assertTrue(any("mermaid.min.js" in p for p in cp.check(root)))

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

    def test_drifted_powershell_version_rejected(self):
        problems = self.check(mise=MISE.replace('powershell = "7.7.0"', 'powershell = "7.6.10"'))
        self.assertTrue(any("powershell" in p and "7.7.0" in p for p in problems))

    def test_powershell_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('powershell = "7.7.0"\n', ""))
        self.assertTrue(any("powershell" in p and "missing" in p for p in problems))

    def test_powershell_bumped_in_script_only_rejected(self):
        problems = self.check(pwsh=PWSH_SCRIPT.replace("version=7.7.0", "version=7.7.1"))
        self.assertTrue(any("powershell" in p for p in problems))

    def test_powershell_malformed_sha_rejected(self):
        bad = PWSH_SCRIPT.replace("sha256_linux_arm64=e59c", "sha256_linux_arm64=XYZ")
        problems = self.check(pwsh=bad)
        self.assertTrue(any("powershell" in p and "sha256" in p for p in problems))

    def test_powershell_script_without_any_sha_rejected(self):
        problems = self.check(pwsh="#!/bin/sh\nversion=7.7.0\n")
        self.assertTrue(any("powershell" in p and "sha256" in p for p in problems))

    def test_powershell_script_missing_rejected(self):
        problems = self.check(pwsh=None)
        self.assertTrue(any("powershell" in p for p in problems))

    def test_drifted_jq_rejected(self):
        problems = self.check(justfile=JUSTFILE.replace('jq_version := "1.8.1"', 'jq_version := "1.8.2"'))
        self.assertTrue(any("jq" in p and "1.8.2" in p for p in problems))

    def test_jq_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('jq = "1.8.1"\n', ""))
        self.assertTrue(any("jq" in p and "missing" in p for p in problems))

    def test_drifted_gh_rejected(self):
        problems = self.check(mise=MISE.replace('gh = "2.104.0"', 'gh = "2.100.0"'))
        self.assertTrue(any("gh" in p and "2.104.0" in p for p in problems))

    def test_gh_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('gh = "2.104.0"\n', ""))
        self.assertTrue(any("gh" in p and "missing" in p for p in problems))

    def test_drifted_python_rejected(self):
        problems = self.check(mise=MISE.replace('python = "3.15.2"', 'python = "3.14.4"'))
        self.assertTrue(any("python" in p and "3.15.2" in p for p in problems))

    def test_python_missing_from_mise_rejected(self):
        problems = self.check(mise=MISE.replace('python = "3.15.2"\n', ""))
        self.assertTrue(any("python" in p and "missing" in p for p in problems))

    def test_python_minor_only_rejected(self):
        problems = self.check(python="3.15\n", mise=MISE.replace('"3.15.2"', '"3.15"'))
        self.assertTrue(any(".python-version" in p and "X.Y.Z" in p for p in problems))

    def test_settings_without_any_rumdl_rejected(self):
        problems = self.check(settings='{"permissions": {"allow": ["Bash(just ci)"]}}')
        self.assertTrue(any("rumdl" in p and "no `uvx rumdl@`" in p for p in problems))

    def test_python_version_file_missing_rejected(self):
        problems = self.check(python=None)
        self.assertTrue(any(".python-version" in p for p in problems))

    def test_real_repository_agrees(self):
        self.assertEqual(cp.check(REPO), [])


if __name__ == "__main__":
    unittest.main()
