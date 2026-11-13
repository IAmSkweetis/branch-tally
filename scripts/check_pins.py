"""Fail when mise.toml drifts from the pins the repository really runs on, or a vendored asset
no longer matches its recorded checksum.

mise.toml is a local convenience. The authorities are the justfile (`<tool>_version`), the CI
workflows (`tool: name@version` for taiki-e/install-action) and rust-toolchain.toml. This script
reads those as text and compares, so it needs neither mise nor just. Usage: check_pins.py [root].
"""

from __future__ import annotations

import hashlib
import re
import sys
import tomllib
from pathlib import Path

# justfile variables that pin a tool: mise tool name -> justfile variable.
JUSTFILE_PINS = {
    "rumdl": "rumdl_version",
    "zizmor": "zizmor_version",
    "gitleaks": "gitleaks_version",
    "git-cliff": "cliff_version",
    "mdbook": "mdbook_version",
    "mdbook-mermaid": "mdbook_mermaid_version",
}
MINISIGN_SCRIPT = Path("scripts/ci-install-minisign.sh")
VENDORED = [Path("docs/manual/svg-pan-zoom.min.js")]


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def justfile_pins(root: Path) -> dict[str, str]:
    text = _read(root / "justfile")
    pins = {}
    for tool, var in JUSTFILE_PINS.items():
        m = re.search(rf'^{var}\s*:=\s*"([^"]+)"', text, re.MULTILINE)
        if m:
            pins[tool] = m.group(1)
    return pins


def workflow_pins(root: Path) -> tuple[dict[str, str], list[str]]:
    """Tool pins from every `tool: a@1,b@2` line in the workflows, and any disagreements."""
    pins: dict[str, str] = {}
    problems: list[str] = []
    for wf in sorted((root / ".github" / "workflows").glob("*.yml")):
        for m in re.finditer(r"^\s*tool:\s*(\S+)\s*$", _read(wf), re.MULTILINE):
            for item in m.group(1).split(","):
                name, _, version = item.partition("@")
                if not version or "${{" in version:
                    continue  # a version read from the justfile at run time
                if pins.setdefault(name, version) != version:
                    problems.append(
                        f"{name}: workflows disagree ({pins[name]} vs {version} in {wf.name})")
    return pins, problems


def minisign_pin(root: Path) -> tuple[str | None, list[str]]:
    """The minisign version pinned in scripts/ci-install-minisign.sh, and any problems with the pin.

    The script is the only place CI gets minisign, pinned by version and archive sha256. The
    sha256 can't be re-derived offline, so it is checked for shape; the version is then compared
    with mise.toml like every other tool.
    """
    path = root / MINISIGN_SCRIPT
    if not path.is_file():
        return None, [f"minisign: {MINISIGN_SCRIPT} is missing"]
    text = _read(path)
    version = re.search(r"^version=(\S+)\s*$", text, re.MULTILINE)
    sha = re.search(r"^sha256=(\S*)\s*$", text, re.MULTILINE)
    problems = []
    if not version or not re.fullmatch(r"\d+\.\d+(\.\d+)?", version.group(1)):
        problems.append(f"minisign: {MINISIGN_SCRIPT} has no well-formed `version=` pin")
    if not sha or not re.fullmatch(r"[0-9a-f]{64}", sha.group(1)):
        problems.append(f"minisign: {MINISIGN_SCRIPT} has no well-formed 64-hex `sha256=` pin")
    return (version.group(1) if version and not problems else None), problems


def mise_tools(root: Path) -> tuple[dict[str, str], list[str]]:
    path = root / "mise.toml"
    if not path.is_file():
        return {}, ["mise.toml is missing"]
    tools = tomllib.loads(_read(path)).get("tools", {})
    out = {}
    for key, value in tools.items():
        version = value.get("version") if isinstance(value, dict) else value
        name = re.split(r"[:/]", key)[-1].lower()  # "aqua:rust-lang/mdBook" -> "mdbook"
        out[name] = str(version)
    return out, []


def check(root: Path) -> list[str]:
    root = Path(root)
    mise, problems = mise_tools(root)
    if problems:
        return problems

    expected: dict[str, tuple[str, str]] = {}  # tool -> (version, where it is pinned)
    for tool, version in justfile_pins(root).items():
        expected[tool] = (version, "the justfile")
    wf, wf_problems = workflow_pins(root)
    problems += wf_problems
    for tool, version in wf.items():
        expected[tool] = (version, ".github/workflows")
    minisign, minisign_problems = minisign_pin(root)
    problems += minisign_problems
    if minisign:
        expected["minisign"] = (minisign, str(MINISIGN_SCRIPT))
    channel = tomllib.loads(_read(root / "rust-toolchain.toml"))["toolchain"]["channel"]
    expected["rust"] = (channel, "rust-toolchain.toml")

    for tool, (version, where) in sorted(expected.items()):
        if tool not in mise:
            problems.append(f"{tool}: missing from mise.toml (pinned {version} in {where})")
        elif mise[tool] != version:
            problems.append(f"{tool}: mise.toml has {mise[tool]} but {where} pins {version}")
    for tool in sorted(set(mise) - set(expected)):
        problems.append(f"{tool}: in mise.toml but pinned nowhere else; add the pin or drop it")

    for asset in VENDORED:
        problems += check_vendored(root, asset)
    return problems


def check_vendored(root: Path, asset: Path) -> list[str]:
    """The asset must hash to the sha256 recorded in `<asset>.sha256` (sha256sum format)."""
    sums = root / (str(asset) + ".sha256")
    if not (root / asset).is_file() or not sums.is_file():
        return [f"{asset}: file or its .sha256 record is missing"]
    recorded = _read(sums).split()[0].lower()
    actual = hashlib.sha256((root / asset).read_bytes()).hexdigest()
    if recorded != actual:
        return [f"{asset}: sha256 is {actual} but the record says {recorded}; "
                "a vendored file changed without its provenance"]
    return []


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    problems = check(root)
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    if not problems:
        print("pins agree: mise.toml matches the justfile, workflows and rust-toolchain.toml; "
              "vendored assets match their checksums")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
