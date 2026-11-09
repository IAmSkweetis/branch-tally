#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Generate the golden outputs of reference/token_report.py for every fixture scenario.

    uv run scripts/make_goldens.py                 # every scenario with a goldens/scenario.json
    uv run scripts/make_goldens.py subagent        # just that scenario
    uv run scripts/make_goldens.py --check         # regenerate in memory, fail on any difference

The reference script takes no flags to point it at fixtures. It reads `~/.claude/projects/` and
asks git for the worktree list. So for each scenario this builds a throwaway home (HOME and
USERPROFILE both set), a throwaway git repository, copies the scenario's slug directories in under
that repository's real slug, creates the real worktrees the manifest lists, and runs the
reference once per (fixture, branch) pair. The reference itself is never edited or imported.

Each scenario directory is a projects root. Its `goldens/scenario.json` says:

    {"repo_slug": "C--repo",
     "worktrees": [{"name": "feat-a", "branch": "feat/a"}],
     "claim_branches": ["feat/a", "main"]}

and this writes, next to it:

    goldens/strict.json   {"<raw gitBranch>": <--strict --json payload>, ...}   one per distinct
                          raw gitBranch in the usage-bearing records ("(none)" when absent)
    goldens/claim.json    {"<branch>": <--json payload>, ...}                     one per
                          claim_branches entry

A pair the reference answers with "No transcript records found" is recorded as
`{"error": "no_records"}`, never dropped. Any other failure aborts the run. Payloads are
normalized: each session `origin` is mapped back to the fixture's own directory name (or "main"),
and the output is checked for the temp paths, slug and user name. Keys are sorted, output is LF
with a trailing newline, so regenerating is a no-op diff.
"""

from __future__ import annotations

import argparse
import copy
import getpass
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
REFERENCE = REPO_ROOT / "reference" / "token_report.py"
DEFAULT_ROOT = REPO_ROOT / "tests" / "fixtures" / "claude-code"
NO_RECORDS = "No transcript records found for branch"
WORKTREE_SLUG = "--claude-worktrees"  # the same fragment the reference matches on
GOLDENS = "goldens"


class GoldenError(Exception):
    """A failure that must stop the run rather than produce a plausible-looking golden."""


def slug_for(path: Path) -> str:
    """Claude Code's slug for a cwd. Replicates the reference's slug_for; not imported from it."""
    return re.sub(r"[^A-Za-z0-9]", "-", str(path))


def substitute_slug(name: str, repo_slug: str, real_slug: str) -> str | None:
    """Replace the fixture's repo_slug prefix with the throwaway repo's slug, or None if the
    directory does not start with it (case-insensitively). A directory that differs from
    repo_slug in case keeps differing: its substituted prefix is lowercased."""
    if not name.lower().startswith(repo_slug.lower()):
        return None
    suffix = name[len(repo_slug):]
    exact = name[: len(repo_slug)] == repo_slug
    return (real_slug if exact else real_slug.lower()) + suffix


def plan_copies(names: list[str], repo_slug: str, real_slug: str) -> dict[str, str]:
    """Fixture directory name -> name to copy it under. Others are copied unchanged."""
    return {n: substitute_slug(n, repo_slug, real_slug) or n for n in names}


def origin_map(plan: dict[str, str]) -> dict[str, str]:
    """The reference reports an origin as the lowercased directory name it found. Map that back
    to the fixture's own name. Two names that differ only in case cannot be told apart."""
    out: dict[str, str] = {}
    for original, copied in plan.items():
        key = copied.lower()
        if key in out and out[key] != original:
            raise GoldenError(f"directories {out[key]!r} and {original!r} differ only in case")
        out[key] = original
    return out


def normalize_payload(payload: dict, origins: dict[str, str]) -> dict:
    out = copy.deepcopy(payload)
    for session in out.get("sessions", []):
        origin = session.get("origin")
        if origin == "main":
            continue
        if origin not in origins:
            raise GoldenError(f"unexpected session origin {origin!r}")
        session["origin"] = origins[origin]
    return out


def parse_result(code: int, stdout: str, stderr: str) -> dict:
    if code != 0:
        if NO_RECORDS in stderr:
            return {"error": "no_records"}
        raise GoldenError(f"reference exited {code}: {stderr.strip()[:500]}")
    try:
        parsed = json.loads(stdout)
    except ValueError as e:
        raise GoldenError(f"reference printed output that is not JSON: {e}") from e
    if not isinstance(parsed, dict):
        raise GoldenError("reference printed JSON that is not an object")
    return parsed


def _is_root_dir(name: str, repo_slug: str) -> bool:
    n, s = name.lower(), repo_slug.lower()
    return n == s or n.startswith(s + WORKTREE_SLUG)


def enumerate_branches(scenario: Path, repo_slug: str) -> list[str]:
    """Distinct raw gitBranch values over the usage-bearing records the reference would read:
    the main directory and its worktree directories, never a prefix sibling or another repo."""
    found: set[str] = set()
    for child in sorted(scenario.iterdir()):
        if not child.is_dir() or child.name == GOLDENS or not _is_root_dir(child.name, repo_slug):
            continue
        for path in sorted(child.rglob("*.jsonl")):
            with path.open(encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        d = json.loads(line)
                    except ValueError:
                        continue
                    msg = d.get("message") if isinstance(d, dict) else None
                    if not isinstance(msg, dict) or not msg.get("usage"):
                        continue
                    if not (d.get("requestId") or d.get("uuid")):
                        continue
                    if str(msg.get("model") or "").startswith("<"):
                        continue
                    branch = d.get("gitBranch") or "(none)"
                    if isinstance(branch, str):
                        found.add(branch)
    return sorted(found)


def load_manifest(scenario: Path) -> dict:
    path = scenario / GOLDENS / "scenario.json"
    try:
        m = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise GoldenError(f"{scenario.name}: cannot read scenario.json: {e}") from e
    ok = (
        isinstance(m, dict)
        and isinstance(m.get("repo_slug"), str) and m["repo_slug"]
        and isinstance(m.get("worktrees", []), list)
        and all(isinstance(w, dict) and isinstance(w.get("name"), str)
                and isinstance(w.get("branch"), str) for w in m.get("worktrees", []))
        and isinstance(m.get("claim_branches", []), list)
        and all(isinstance(b, str) for b in m.get("claim_branches", []))
    )
    if not ok:
        raise GoldenError(f"{scenario.name}: scenario.json does not match the manifest contract")
    return m


def _git(args: list[str], cwd: Path, env: dict[str, str]) -> None:
    p = subprocess.run(
        ["git", "-c", "user.name=goldens", "-c", "user.email=goldens@example.invalid",
         "-c", "commit.gpgsign=false", *args],
        cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8")
    if p.returncode != 0:
        raise GoldenError(f"git {args[0]} failed: {p.stderr.strip()[:500]}")


def _run_reference(repo: Path, env: dict[str, str], branch: str, strict: bool) -> dict:
    cmd = [sys.executable, str(REFERENCE), branch, "--repo", str(repo), "--json"]
    if strict:
        cmd.append("--strict")
    p = subprocess.run(cmd, cwd=repo, env=env, capture_output=True, text=True, encoding="utf-8")
    return parse_result(p.returncode, p.stdout, p.stderr)


def _rmtree(path: Path) -> None:
    def retry(func, p, *_):  # git objects are read-only on Windows
        os.chmod(p, stat.S_IWRITE)
        func(p)
    kw = {"onexc": retry} if sys.version_info >= (3, 12) else {"onerror": retry}
    shutil.rmtree(path, **kw)


def generate(scenario: Path) -> tuple[dict, dict]:
    """Build the throwaway world for one scenario and return (strict, claim) golden dicts."""
    if sys.version_info < (3, 11):
        raise GoldenError("the reference needs Python 3.11+; run this with uv")
    manifest = load_manifest(scenario)
    repo_slug = manifest["repo_slug"]
    strict_branches = enumerate_branches(scenario, repo_slug)
    tmp = Path(tempfile.mkdtemp(prefix="bt-goldens-")).resolve()
    try:
        home, repo = tmp / "home", tmp / "work" / "repo"
        repo.mkdir(parents=True)
        projects = home / ".claude" / "projects"
        projects.mkdir(parents=True)
        env = {**os.environ, "HOME": str(home), "USERPROFILE": str(home),
               "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
               "PYTHONUTF8": "1", "PYTHONIOENCODING": "utf-8"}
        _git(["init", "-q", "-b", "main"], repo, env)
        _git(["commit", "-q", "--allow-empty", "-m", "init"], repo, env)
        for wt in manifest.get("worktrees", []):
            _git(["worktree", "add", "-q", "-b", wt["branch"],
                  str(repo / ".claude" / "worktrees" / wt["name"])], repo, env)

        real_slug = slug_for(repo.resolve())
        names = sorted(c.name for c in scenario.iterdir() if c.is_dir() and c.name != GOLDENS)
        plan = plan_copies(names, repo_slug, real_slug)
        origins = origin_map(plan)
        for original, copied in plan.items():
            shutil.copytree(scenario / original, projects / copied)

        strict = {b: normalize_payload(_run_reference(repo, env, b, True), origins)
                  for b in strict_branches}
        claim = {b: normalize_payload(_run_reference(repo, env, b, False), origins)
                 for b in manifest.get("claim_branches", [])}

        text = (render(strict) + render(claim)).lower()
        needles = {str(tmp), tmp.as_posix(), str(home), real_slug, str(repo),
                   slug_for(tmp), tmp.name}
        user = getpass.getuser()
        if len(user) >= 4:
            needles.add(user)
        for needle in needles:
            if needle.lower() in text.replace("\\\\", "\\"):
                raise GoldenError(f"{scenario.name}: a machine-specific value leaked into a golden")
        return strict, claim
    finally:
        _rmtree(tmp)


def render(obj: dict) -> str:
    return json.dumps(obj, indent=2, sort_keys=True) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Generate goldens from reference/token_report.py.")
    ap.add_argument("scenarios", nargs="*", help="scenario names (default: all with a manifest)")
    ap.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="fixture root to scan")
    ap.add_argument("--check", action="store_true",
                    help="write nothing; fail if the goldens on disk differ from a regeneration")
    args = ap.parse_args(argv)
    try:
        return _run(args)
    except GoldenError as e:
        print(f"make_goldens: error: {e}", file=sys.stderr)
        return 1


def _run(args: argparse.Namespace) -> int:
    root: Path = args.root
    if not root.is_dir():
        raise GoldenError(f"no fixture directory at {root}")
    dirs = sorted(d for d in root.iterdir() if d.is_dir())
    have = {d.name: d for d in dirs if (d / GOLDENS / "scenario.json").is_file()}
    for d in dirs:
        if d.name not in have and (not args.scenarios or d.name in args.scenarios):
            print(f"make_goldens: no {GOLDENS}/scenario.json: {d.name} (no goldens made)",
                  file=sys.stderr)
    wanted = args.scenarios or list(have)
    missing = [n for n in wanted if n not in have]
    if missing and args.scenarios:
        raise GoldenError(f"no scenario with a manifest named: {', '.join(missing)}")
    if not wanted:
        raise GoldenError(f"no scenario under {root} has a {GOLDENS}/scenario.json")

    stale = 0
    for name in wanted:
        strict, claim = generate(have[name])
        for fname, data in (("strict.json", strict), ("claim.json", claim)):
            path = have[name] / GOLDENS / fname
            body = render(data).encode("utf-8")
            if args.check:
                if not path.is_file() or path.read_bytes() != body:
                    print(f"make_goldens: STALE {name}/{GOLDENS}/{fname}", file=sys.stderr)
                    stale += 1
            else:
                path.write_bytes(body)
        print(f"make_goldens: {name}: {len(strict)} strict, {len(claim)} claim", file=sys.stderr)
    if stale:
        print(f"make_goldens: {stale} golden file(s) differ; run `just goldens`", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
