#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Scrub real Claude Code transcripts into fixtures that are safe to commit.

    uv run scripts/scrub_transcripts.py --root D:\\work\\myrepo  SRC  DEST

SRC is a slug directory (holding `<session>.jsonl` and `<session>/subagents/agent-<id>.jsonl`)
or a whole projects root of slug directories. DEST must not exist or must be empty. `--root` is
the repository root as the transcripts recorded it, in either path form. It is the one thing the
tool cannot guess, and every `cwd` under it becomes a neutral root.

What survives is exactly what branch-tally reads (Governance III, docs/references/
transcript_format.md): `type`, `requestId`, `uuid`, `sessionId`, `agentId`, `isSidechain`,
`timestamp`, `cwd`, `gitBranch`, `attributionSkill`, `attributionAgent`, `message.model`, and the
numeric fields of `message.usage`. `message.content` becomes "PLACEHOLDER". Every other key is
dropped, never copied. Ids become short deterministic placeholders (equal in, equal out), and files
and directories are renamed to match. A line that is not JSON, or whose kept fields have an
unexpected type, is dropped and only counted. Nothing from a transcript is ever printed.

The run ends with a self-check that re-reads DEST and fails, deleting DEST, if any string value
outside the allowlist survives.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

PLACEHOLDER = "PLACEHOLDER"

TYPE_RE = re.compile(r"^[A-Za-z_-]{1,40}$")
TIMESTAMP_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?$"
)
BRANCH_RE = re.compile(r"^[A-Za-z0-9._/@+#()-]{0,200}$")
ATTRIBUTION_RE = re.compile(r"^[\w:./@-]{1,100}$")
MODEL_RE = re.compile(r"^[\w.\[\]<>:-]{1,100}$")
NEUTRAL_CWD_RE = re.compile(r"^(C:\\(repo|other)|/(repo|other))([\\/].*)?$")

ID_PATTERNS = {
    "requestId": re.compile(r"^(req_\d+)?$"),
    "uuid": re.compile(r"^(u\d+)?$"),
    "sessionId": re.compile(r"^(s\d+)?$"),
    "agentId": re.compile(r"^(a\d+)?$"),
}

USAGE_INTS = (
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
)
USAGE_NESTED = {
    "cache_creation": ("ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"),
    "output_tokens_details": ("thinking_tokens",),
}
TOP_KEYS = (
    {"type", "timestamp", "cwd", "gitBranch", "isSidechain", "attributionSkill",
     "attributionAgent", "message"} | set(ID_PATTERNS)
)


class ScrubError(Exception):
    """A usage or safety failure. Messages never contain transcript content."""


class Unscrubbable(Exception):
    """One line has a kept field of an unexpected shape. Carries no content."""


@dataclass
class Stats:
    files_written: int = 0
    records_written: int = 0
    malformed_lines: int = 0
    unscrubbable_lines: int = 0
    files_ignored: int = 0


def _convert(path: str) -> str:
    """Forward slashes, and `X:` as `/x`, so both recorded forms of a path compare equal."""
    s = path.replace("\\", "/")
    m = re.match(r"^([A-Za-z]):(/.*)?$", s)
    if m:
        s = "/" + m.group(1) + (m.group(2) or "")
    return s.rstrip("/") if len(s) > 1 else s


def normalize_root(root: str) -> str:
    return _convert(root).lower()


def rewrite_cwd(cwd: str, root_norm: str) -> str:
    """Put a cwd under a neutral root, keeping its separator form and its subpath."""
    windows = "\\" in cwd or re.match(r"^[A-Za-z]:", cwd) is not None
    conv = _convert(cwd)
    if conv.lower() == root_norm or conv.lower().startswith(root_norm + "/"):
        base, rest = "repo", conv[len(root_norm):]
    else:
        base, rest = "other", ""
    if windows:
        return "C:\\" + base + rest.replace("/", "\\")
    return "/" + base + rest


def slug_of(path_text: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "-", path_text)


class IdMap:
    """Deterministic short placeholders: equal ids in give equal ids out."""

    PREFIX = {"requestId": "req_", "uuid": "u", "sessionId": "s", "agentId": "a"}

    def __init__(self) -> None:
        self._maps: dict[str, dict[str, str]] = {k: {} for k in self.PREFIX}

    def get(self, kind: str, real: str) -> str:
        m = self._maps[kind]
        if real not in m:
            m[real] = f"{self.PREFIX[kind]}{len(m) + 1}"
        return m[real]


def _int_ok(v: object) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and v >= 0


def _scrub_usage(usage: object) -> dict:
    if not isinstance(usage, dict):
        raise Unscrubbable
    out: dict = {}
    for key in USAGE_INTS:
        if usage.get(key) is not None:
            if not _int_ok(usage[key]):
                raise Unscrubbable
            out[key] = usage[key]
    for key, subkeys in USAGE_NESTED.items():
        sub = usage.get(key)
        if sub is None:
            continue
        if not isinstance(sub, dict):
            raise Unscrubbable
        kept = {}
        for sk in subkeys:
            if sub.get(sk) is not None:
                if not _int_ok(sub[sk]):
                    raise Unscrubbable
                kept[sk] = sub[sk]
        out[key] = kept
    return out


def _scrub_record(d: object, ids: IdMap, root_norm: str) -> dict:
    if not isinstance(d, dict):
        raise Unscrubbable
    out: dict = {}

    def text(key: str, pattern: re.Pattern[str]) -> str | None:
        v = d.get(key)
        if v is None:
            return None
        if not isinstance(v, str) or not pattern.match(v):
            raise Unscrubbable
        return v

    t = text("type", TYPE_RE)
    if t is not None:
        out["type"] = t
    for kind in ("requestId", "uuid", "sessionId", "agentId"):
        v = d.get(kind)
        if v is None:
            continue
        if not isinstance(v, str):
            raise Unscrubbable
        out[kind] = ids.get(kind, v) if v else ""
    side = d.get("isSidechain")
    if side is not None:
        if not isinstance(side, bool):
            raise Unscrubbable
        out["isSidechain"] = side
    ts = text("timestamp", TIMESTAMP_RE)
    if ts is not None:
        out["timestamp"] = ts
    cwd = d.get("cwd")
    if cwd is not None:
        if not isinstance(cwd, str):
            raise Unscrubbable
        out["cwd"] = rewrite_cwd(cwd, root_norm)
    for key, pattern in (("gitBranch", BRANCH_RE), ("attributionSkill", ATTRIBUTION_RE),
                         ("attributionAgent", ATTRIBUTION_RE)):
        v = text(key, pattern)
        if v is not None:
            out[key] = v
    msg = d.get("message")
    if msg is not None:
        if not isinstance(msg, dict):
            raise Unscrubbable
        m: dict = {}
        model = msg.get("model")
        if model is not None:
            if not isinstance(model, str) or not MODEL_RE.match(model):
                raise Unscrubbable
            m["model"] = model
        m["content"] = PLACEHOLDER
        if msg.get("usage") is not None:
            m["usage"] = _scrub_usage(msg["usage"])
        out["message"] = m
    return out


def scrub_line(raw: bytes, ids: IdMap, root_norm: str) -> dict:
    """Raises json/Unicode errors for malformed input and Unscrubbable for a bad shape."""
    return _scrub_record(json.loads(raw.decode("utf-8")), ids, root_norm)


def _slug_dirs(src: Path) -> list[Path]:
    if any(p.suffix == ".jsonl" for p in src.iterdir() if p.is_file()):
        return [src]
    return sorted(p for p in src.iterdir() if p.is_dir())


def _neutral_slug_name(name: str, root: str, root_norm: str, others: dict[str, str]) -> str:
    """Rename a slug dir. Dirs under the root keep their suffix (worktree, sibling)."""
    drive = re.match(r"^/([a-z])(/.*)?$", root_norm)
    candidates = []
    if drive:
        win = slug_of(drive.group(1) + ":" + (drive.group(2) or "").replace("/", "\\"))
        candidates.append((win, "C--repo"))
        candidates.append((slug_of(root_norm), "-repo"))
    else:
        candidates.append((slug_of(root_norm), "-repo"))
    for prefix, neutral in candidates:
        if name.lower().startswith(prefix.lower()):
            exact = slug_of(root)[: len(prefix)] == name[: len(prefix)]
            return (neutral if exact else neutral.lower()) + name[len(prefix):]
    if name not in others:
        others[name] = "C--other" if not others else f"C--other-{len(others) + 1}"
    return others[name]


def scrub_tree(src: Path, dest: Path, root: str) -> Stats:
    src, dest = Path(src).resolve(), Path(dest).resolve()
    if not src.is_dir():
        raise ScrubError("source is not a directory")
    if dest == src or src in dest.parents or dest in src.parents:
        raise ScrubError("source and destination must not contain each other")
    if dest.exists() and (not dest.is_dir() or any(dest.iterdir())):
        raise ScrubError("destination exists and is not empty")
    root_norm = normalize_root(root)
    ids, stats, others = IdMap(), Stats(), {}
    outputs: dict[Path, list[str]] = {}

    def scrub_file(path: Path, out_path: Path) -> None:
        if out_path in outputs:
            raise ScrubError("two source files map to one destination file")
        lines = outputs[out_path] = []
        for raw in path.read_bytes().split(b"\n"):
            if not raw.strip():
                continue
            try:
                lines.append(json.dumps(scrub_line(raw, ids, root_norm), separators=(",", ":")))
                stats.records_written += 1
            except (ValueError, UnicodeDecodeError):  # JSONDecodeError is a ValueError
                stats.malformed_lines += 1
            except Unscrubbable:
                stats.unscrubbable_lines += 1

    for slug_dir in _slug_dirs(src):
        out_slug = dest / _neutral_slug_name(slug_dir.name, root, root_norm, others)
        for entry in sorted(slug_dir.iterdir(), key=lambda p: p.name):
            if entry.is_file() and entry.suffix == ".jsonl":
                scrub_file(entry, out_slug / (ids.get("sessionId", entry.stem) + ".jsonl"))
            elif entry.is_dir() and (entry / "subagents").is_dir():
                session = ids.get("sessionId", entry.name)
                for agent in sorted((entry / "subagents").iterdir(), key=lambda p: p.name):
                    m = re.match(r"^agent-(.+)\.jsonl$", agent.name)
                    if agent.is_file() and m:
                        target = out_slug / session / "subagents" / (
                            "agent-" + ids.get("agentId", m.group(1)) + ".jsonl")
                        scrub_file(agent, target)
                    else:
                        stats.files_ignored += 1
            else:
                stats.files_ignored += 1

    for out_path, lines in outputs.items():
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_bytes(("\n".join(lines) + "\n" if lines else "").encode("ascii"))
        stats.files_written += 1

    print(
        f"scrub: {stats.records_written} records in {stats.files_written} files written; "
        f"dropped: {stats.malformed_lines} malformed lines, {stats.unscrubbable_lines} lines with "
        f"an unexpected field shape, {stats.files_ignored} files or dirs ignored",
        file=sys.stderr,
    )
    return stats


def _check_value(obj: object, path: str, root_norm: str, problems: list[str]) -> None:
    """Allowlist walk. Reports key paths only, never values."""
    if not isinstance(obj, dict):
        problems.append(f"{path or '<line>'}: not an object")
        return
    for key, val in obj.items():
        here = f"{path}.{key}" if path else key
        if not path:
            if key not in TOP_KEYS:
                problems.append(f"{here}: key outside the allowlist")
            elif key == "message":
                _check_message(val, here, problems)
            elif key in ID_PATTERNS:
                if not isinstance(val, str) or not ID_PATTERNS[key].match(val):
                    problems.append(f"{here}: not a placeholder id")
            elif key == "isSidechain":
                if not isinstance(val, bool):
                    problems.append(f"{here}: not a bool")
            elif key == "cwd":
                if (not isinstance(val, str) or not NEUTRAL_CWD_RE.match(val)
                        or root_norm in _convert(val).lower()):
                    problems.append(f"{here}: not under the neutral root")
            else:
                pattern = {"type": TYPE_RE, "timestamp": TIMESTAMP_RE, "gitBranch": BRANCH_RE,
                           "attributionSkill": ATTRIBUTION_RE,
                           "attributionAgent": ATTRIBUTION_RE}[key]
                if not isinstance(val, str) or not pattern.match(val):
                    problems.append(f"{here}: string fails its pattern")


def _check_message(msg: object, path: str, problems: list[str]) -> None:
    if not isinstance(msg, dict):
        problems.append(f"{path}: not an object")
        return
    for key, val in msg.items():
        here = f"{path}.{key}"
        if key == "content":
            if val != PLACEHOLDER:
                problems.append(f"{here}: not the placeholder")
        elif key == "model":
            if not isinstance(val, str) or not MODEL_RE.match(val):
                problems.append(f"{here}: string fails its pattern")
        elif key == "usage":
            _check_usage(val, here, problems)
        else:
            problems.append(f"{here}: key outside the allowlist")


def _check_usage(usage: object, path: str, problems: list[str]) -> None:
    if not isinstance(usage, dict):
        problems.append(f"{path}: not an object")
        return
    for key, val in usage.items():
        here = f"{path}.{key}"
        if key in USAGE_INTS:
            if not _int_ok(val):
                problems.append(f"{here}: not a non-negative integer")
        elif key in USAGE_NESTED:
            if not isinstance(val, dict):
                problems.append(f"{here}: not an object")
                continue
            for sk, sv in val.items():
                if sk not in USAGE_NESTED[key] or not _int_ok(sv):
                    problems.append(f"{here}.{sk}: outside the allowlist or not an integer")
        else:
            problems.append(f"{here}: key outside the allowlist")


NAME_PATTERNS = (
    re.compile(r"^s\d+\.jsonl$"),
    re.compile(r"^s\d+$"),
    re.compile(r"^subagents$"),
    re.compile(r"^agent-a\d+\.jsonl$"),
)


def self_check(dest: Path, root_norm: str) -> list[str]:
    """Re-read DEST and list every way it falls outside the allowlist. Empty means clean."""
    problems: list[str] = []
    dest = Path(dest)
    for p in sorted(dest.rglob("*")):
        rel = p.relative_to(dest)
        # The first component is a slug dir: letters, digits and dashes only.
        for i, part in enumerate(rel.parts):
            if i == 0:
                if not re.match(r"^[A-Za-z0-9-]+$", part):
                    problems.append(f"{rel}: slug directory name")
            elif not any(pat.match(part) for pat in NAME_PATTERNS):
                problems.append(f"{rel}: unexpected file or directory name")
        if p.is_file():
            if p.suffix != ".jsonl":
                problems.append(f"{rel}: not a .jsonl file")
                continue
            for n, line in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
                try:
                    obj = json.loads(line)
                except ValueError:
                    problems.append(f"{rel}:{n}: not JSON")
                    continue
                found: list[str] = []
                _check_value(obj, "", root_norm, found)
                problems.extend(f"{rel}:{n}: {f}" for f in found)
    return problems


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Scrub Claude Code transcripts into fixtures.")
    ap.add_argument("src", type=Path, help="a slug directory, or a projects root of them")
    ap.add_argument("dest", type=Path, help="output directory (absent or empty)")
    ap.add_argument("--root", required=True,
                    help="the repository root as recorded in the transcripts (either path form)")
    args = ap.parse_args(argv)
    try:
        scrub_tree(args.src, args.dest, args.root)
        problems = self_check(args.dest, normalize_root(args.root))
    except ScrubError as e:
        print(f"scrub: error: {e}", file=sys.stderr)
        return 1
    if problems:
        shutil.rmtree(args.dest, ignore_errors=True)
        print(f"scrub: self-check FAILED, destination removed ({len(problems)} problems):",
              file=sys.stderr)
        for line in problems[:20]:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("scrub: self-check passed", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
