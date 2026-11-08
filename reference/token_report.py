#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Aggregate Claude Code token usage for a git branch across every session that worked on it.

Claude Code writes one JSON record per API call to ~/.claude/projects/<cwd-slug>/, including
subagent transcripts under <session-id>/subagents/. Each record carries gitBranch, so usage for
a branch can be recovered locally with no OTEL collector and no telemetry export.

This project runs agents in git worktrees under .claude/worktrees/. A worktree entered as its own
Claude Code session (its own cwd) gets its own ~/.claude/projects/ directory, and every one of
those is scanned too. But a subagent dispatched via the Agent tool into a worktree does NOT get a
separate session or transcript directory -- its records land under the *main checkout's* slug, in
<session-id>/subagents/, with gitBranch stamped live from that worktree's HEAD at call time
(typically the harness-generated `worktree-agent-<id>` branch, before the agent has cut its real
feature branch). Recovering those records means matching on the gitBranch string itself, not on
which transcript directory they were found in.

    uv run scripts/token_report.py                       # print the table for the current branch
    uv run scripts/token_report.py --pr                  # splice it into this branch's open PR
    uv run scripts/token_report.py --pr 7 --dry-run      # show the resulting body, change nothing
    uv run scripts/token_report.py feat/catch-window     # explicit branch
    uv run scripts/token_report.py --json                # machine-readable
    uv run scripts/token_report.py --no-cost             # drop the estimated-cost column
    uv run scripts/token_report.py --strict              # naive gitBranch filter, no recovery
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

# Branch prefixes that identify a discrete unit of work, used by the session-claim rule below.
# These are the Conventional Branch types this project allows (docs/references/dev_workflow.md),
# including the `feature/` and `bugfix/` spellings that page accepts as equivalents.
FEATURE_RE = re.compile(r"^(feat|feature|fix|bugfix|hotfix|chore|release)/")

# Branches Claude Code generates for its own worktrees. They name no unit of work, so a record
# carrying one is a candidate for recovery onto the branch the worktree actually holds.
WORKTREE_BRANCH_RE = re.compile(r"^worktree-agent-")

# Slug fragment that separates a worktree's transcript directory from the main checkout's.
WORKTREE_SLUG = "--claude-worktrees"

# $/MTok list rates. Cache write 5m = 1.25x input, 1h = 2x input; cache read = 0.1x input
# except where CACHE_READ_RATE overrides it.
PRICES: dict[str, tuple[float, float]] = {
    "claude-fable-5-1": (10.0, 50.0),
    "claude-fable-5": (10.0, 50.0),
    "claude-opus-5": (5.0, 25.0),
    "claude-opus-4-8": (5.0, 25.0),
    "claude-opus-4-7": (5.0, 25.0),
    "claude-opus-4-6": (5.0, 25.0),
    "claude-sonnet-5": (2.0, 10.0),
    "claude-sonnet-4-6": (3.0, 15.0),
    "claude-haiku-4-5": (1.0, 5.0),
}

# Models whose cache reads are not the usual 0.1x of the input rate, in $/MTok.
CACHE_READ_RATE: dict[str, float] = {
    "claude-fable-5-1": 0.25,
}

START = "<!-- token-report:start -->"
END = "<!-- token-report:end -->"
HEADING = "## Token usage"


def price_for(model: str) -> tuple[float, float] | None:
    if model in PRICES:
        return PRICES[model]
    # Model ids pick up suffixes (`claude-opus-5[1m]`), so fall back to a prefix match --
    # longest key first, or `claude-fable-5-1` would be shadowed by `claude-fable-5`.
    for known in sorted(PRICES, key=len, reverse=True):
        if model.startswith(known):
            return PRICES[known]
    return None


def cache_read_rate(model: str, input_rate: float) -> float:
    for known in sorted(CACHE_READ_RATE, key=len, reverse=True):
        if model.startswith(known):
            return CACHE_READ_RATE[known]
    return input_rate * 0.1


@dataclass
class Record:
    session: str
    branch: str
    ts: str
    model: str
    sidechain: bool
    attr: str
    origin: str
    input: int
    w5: int
    w1h: int
    read: int
    output: int
    think: int

    @property
    def tokens(self) -> int:
        return self.input + self.w5 + self.w1h + self.read + self.output


@dataclass
class Bucket:
    input: int = 0
    w5: int = 0
    w1h: int = 0
    read: int = 0
    output: int = 0
    think: int = 0
    calls: int = 0

    def add(self, r: Record) -> None:
        self.input += r.input
        self.w5 += r.w5
        self.w1h += r.w1h
        self.read += r.read
        self.output += r.output
        self.think += r.think
        self.calls += 1

    def cost(self, model: str) -> float | None:
        rates = price_for(model)
        if rates is None:
            return None
        cin, cout = rates
        return (
            self.input * cin
            + self.w5 * cin * 1.25
            + self.w1h * cin * 2
            + self.read * cache_read_rate(model, cin)
            + self.output * cout
        ) / 1_000_000


@dataclass
class Session:
    first: str
    last: str
    origin: str
    tokens: int = 0
    calls: int = 0
    agents: set[str] = field(default_factory=set)

    @property
    def who(self) -> str:
        return ", ".join(sorted(self.agents)) if self.agents else "—"


def slug_for(path: Path) -> str:
    """Claude Code slugifies the cwd into a directory name under ~/.claude/projects/."""
    return re.sub(r"[^A-Za-z0-9]", "-", str(path))


def worktree_branches(repo: Path) -> dict[str, str]:
    """Map each of this repository's worktrees to the branch it holds, keyed two ways.

    A worktree session records `worktree-agent-<id>` as its gitBranch, which names no unit of
    work -- so those records can only be attributed by asking git which branch the worktree that
    produced them is on. That is a snapshot of *now*: a worktree reused for a second branch
    reports the later one, and a removed worktree cannot be recovered at all. `--strict` turns
    this off.

    Two different things carry that gitBranch stamp, so this dict is keyed both ways:

    - A worktree entered as its own Claude Code session slugs its cwd into its own transcript
      directory. Keyed by that slug, for records whose gitBranch is "(none)" -- origin is the
      only signal available for those.
    - A subagent dispatched via the Agent tool into a worktree leaves its records under the
      *main* checkout's slug instead, with gitBranch literally `worktree-agent-<id>`. That string
      is itself the key here, so it recovers regardless of which transcript directory the record
      was found in.
    """
    out: dict[str, str] = {}
    p = subprocess.run(["git", "worktree", "list", "--porcelain"], cwd=repo,
                       capture_output=True, text=True)
    if p.returncode != 0:
        return out
    path: Path | None = None
    for line in p.stdout.splitlines():
        if line.startswith("worktree "):
            path = Path(line[len("worktree "):]).resolve()
        elif line.startswith("branch refs/heads/") and path is not None:
            branch = line[len("branch refs/heads/"):]
            out[slug_for(path).lower()] = branch
            out[f"worktree-{path.name}"] = branch
            path = None
    return out


def transcript_roots(repo: Path) -> tuple[list[Path], str]:
    """Every transcript directory for this repository: the main checkout plus each worktree.

    Worktrees live under <repo>/.claude/worktrees/<name>, so their slugs are the repo slug plus
    a fixed suffix. Requiring that suffix keeps a sibling repository whose path merely starts
    the same way (dodgeball vs dodgeball-game) out of the totals.
    """
    projects = Path.home() / ".claude" / "projects"
    if not projects.is_dir():
        sys.exit(f"No Claude Code transcript directory at {projects}")

    slug = slug_for(repo).lower()
    prefix = slug + WORKTREE_SLUG
    roots = [
        child for child in sorted(projects.iterdir())
        if child.is_dir()
        and (child.name.lower() == slug or child.name.lower().startswith(prefix))
    ]
    if not roots:
        sys.exit(f"No transcript directory for {repo} (expected slug {slug})")
    return roots, slug


def load_records(roots: list[Path], main_slug: str, wt_branches: dict[str, str],
                 strict: bool) -> list[Record]:
    """Read every usage-bearing record once. Resumed sessions replay history, so dedup on
    requestId (falling back to the record uuid) or totals inflate by roughly 2x. The dedup set
    spans all roots: the same session can be resumed from a different checkout."""
    seen: set[str] = set()
    out: list[Record] = []
    for root in roots:
        name = root.name.lower()
        origin = "main" if name == main_slug else name
        recovered = None if strict else wt_branches.get(name)
        for path in sorted(root.rglob("*.jsonl")):
            with path.open(encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        d = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    msg = d.get("message") or {}
                    usage = msg.get("usage")
                    if not usage:
                        continue
                    key = d.get("requestId") or d.get("uuid")
                    if not key or key in seen:
                        continue
                    # Claude Code emits <synthetic> assistant messages for interrupts and local
                    # errors. They carry a zero usage block and no model, so they only add noise.
                    if str(msg.get("model") or "").startswith("<"):
                        continue
                    seen.add(key)

                    branch = d.get("gitBranch") or "(none)"
                    # A worktree's own branch names no unit of work; git says what it holds.
                    # A `worktree-agent-<id>` stamp identifies the worktree by itself, so it is
                    # recovered by that exact string -- whatever transcript directory (origin)
                    # the record was found in, since an Agent-tool dispatch's records land under
                    # the main checkout's slug rather than a slug of their own. "(none)" carries
                    # no such identifying string, so it can only fall back to origin.
                    if not strict:
                        if WORKTREE_BRANCH_RE.match(branch) and branch in wt_branches:
                            branch = wt_branches[branch]
                        elif recovered and branch == "(none)":
                            branch = recovered

                    cw = usage.get("cache_creation") or {}
                    w5 = cw.get("ephemeral_5m_input_tokens")
                    if w5 is None:
                        w5 = usage.get("cache_creation_input_tokens") or 0
                    sidechain = bool(d.get("isSidechain"))
                    attr = d.get("attributionSkill")
                    if not attr:
                        agent = d.get("attributionAgent")
                        attr = f"agent:{agent or 'subagent'}" if sidechain else "main"

                    details = usage.get("output_tokens_details") or {}
                    out.append(
                        Record(
                            session=d.get("sessionId") or "",
                            branch=branch,
                            ts=d.get("timestamp") or "",
                            model=msg.get("model") or "unknown",
                            sidechain=sidechain,
                            attr=attr,
                            origin=origin,
                            input=usage.get("input_tokens") or 0,
                            w5=w5,
                            w1h=cw.get("ephemeral_1h_input_tokens") or 0,
                            read=usage.get("cache_read_input_tokens") or 0,
                            output=usage.get("output_tokens") or 0,
                            think=details.get("thinking_tokens") or 0,
                        )
                    )
    return out


def select(records: list[Record], branch: str, strict: bool) -> list[Record]:
    """Pick the records belonging to `branch`.

    A plain `gitBranch == branch` filter under-reports, because work on a branch does not all
    happen on the branch: a session commonly starts on main before the branch is cut. So a
    session is CLAIMED by a branch when that is the only work-shaped branch it ever touched, and
    then all of its records count regardless of the branch they were recorded under. Sessions
    that touched two work branches stay ambiguous and fall back to the strict filter.
    """
    if strict:
        return [r for r in records if r.branch == branch]

    features: dict[str, set[str]] = defaultdict(set)
    for r in records:
        if FEATURE_RE.match(r.branch):
            features[r.session].add(r.branch)
    claimed = {s for s, brs in features.items() if brs == {branch}}
    return [r for r in records if r.session in claimed or r.branch == branch]


def summarize(
    mine: list[Record],
) -> tuple[dict[str, Bucket], dict[str, Bucket], dict[str, Session]]:
    by_model: dict[str, Bucket] = defaultdict(Bucket)
    by_attr: dict[str, Bucket] = defaultdict(Bucket)
    sessions: dict[str, Session] = {}
    for r in mine:
        by_model[r.model].add(r)
        by_attr[r.attr].add(r)
        s = sessions.get(r.session)
        if s is None:
            s = sessions[r.session] = Session(first=r.ts, last=r.ts, origin=r.origin)
        s.first = min(s.first, r.ts)
        s.last = max(s.last, r.ts)
        s.tokens += r.tokens
        s.calls += 1
        if r.attr.startswith("agent:"):
            s.agents.add(r.attr[len("agent:"):])
    return by_model, by_attr, sessions


def render(branch: str, mine: list[Record], strict: bool, show_cost: bool = True) -> str:
    by_model, by_attr, sessions = summarize(mine)
    sub_calls = sum(1 for r in mine if r.sidechain)
    off_branch = sum(r.tokens for r in mine if r.branch != branch)
    worktree_sessions = sum(1 for s in sessions.values() if s.origin != "main")

    rows = sorted(by_model.items(), key=lambda kv: -kv[1].output)
    total = Bucket()
    for _, b in rows:
        for f in ("input", "w5", "w1h", "read", "output", "think", "calls"):
            setattr(total, f, getattr(total, f) + getattr(b, f))
    total_cost = sum(b.cost(m) or 0.0 for m, b in rows)
    ordered = sorted(sessions.items(), key=lambda kv: kv[1].first)

    def n(x: int) -> str:
        return f"{x:,}"

    def span(s: Session) -> str:
        return f"{s.first[5:16].replace('T', ' ')} → {s.last[5:16].replace('T', ' ')}"

    cost_head, cost_rule = (" Est. cost |", "--:|") if show_cost else ("", "")
    o: list[str] = []
    o.append(
        "| Model | Calls | Input | Cache write | Cache read | Output | (thinking) |" + cost_head
    )
    o.append("|---|--:|--:|--:|--:|--:|--:|" + cost_rule)
    for model, b in rows:
        c = b.cost(model)
        cell = f" {'—' if c is None else f'${c:.2f}'} |" if show_cost else ""
        o.append(
            f"| `{model}` | {n(b.calls)} | {n(b.input)} | {n(b.w5 + b.w1h)} | {n(b.read)} "
            f"| {n(b.output)} | {n(b.think)} |" + cell
        )
    o.append(
        f"| **Total** | **{n(total.calls)}** | **{n(total.input)}** | "
        f"**{n(total.w5 + total.w1h)}** | **{n(total.read)}** | **{n(total.output)}** | "
        f"**{n(total.think)}** |"
        + (f" **${total_cost:.2f}** |" if show_cost else "")
    )
    o.append("")

    o.append("| Session | Where | Agents | Span (UTC) | Calls | Tokens |")
    o.append("|---|---|---|---|--:|--:|")
    for sid, s in ordered:
        where = "main checkout" if s.origin == "main" else "worktree"
        o.append(
            f"| `{sid[:8]}` | {where} | {s.who} | {span(s)} | {n(s.calls)} "
            f"| {s.tokens / 1e6:.1f}M |"
        )
    o.append("")

    note = (
        f"<sub>Branch `{branch}` · {len(ordered)} session(s), {worktree_sessions} in a "
        f"worktree · {n(sub_calls)} of {n(total.calls)} calls from subagents · "
        f"{off_branch / 1e6:.1f}M tokens recovered from pre-branch/worktree checkouts"
    )
    note += (
        " · est. cost at list API rates, not amounts billed</sub>" if show_cost else "</sub>"
    )
    o.append(note)
    o.append("")

    o.append("<details><summary>Breakdown by skill / agent</summary>")
    o.append("")
    o.append("| Attribution | Calls | Tokens |")
    o.append("|---|--:|--:|")
    for attr, b in sorted(
        by_attr.items(),
        key=lambda kv: -(kv[1].input + kv[1].w5 + kv[1].w1h + kv[1].read + kv[1].output),
    ):
        tok = b.input + b.w5 + b.w1h + b.read + b.output
        o.append(f"| {attr} | {n(b.calls)} | {n(tok)} |")
    o.append("")
    o.append("</details>")
    return "\n".join(o)


def wrap(report: str) -> str:
    """Markers are owned here, not by render(), so a report can never be spliced unmarked --
    which would make the next run append a second table instead of replacing this one."""
    return f"{START}\n{report}\n{END}"


def gh(args: list[str], repo: Path, stdin: str | None = None) -> str:
    # encoding= is required, not cosmetic: text=True alone encodes stdin with the locale codec,
    # which is cp1252 on Windows. The report's arrows and middots then kill subprocess's writer
    # thread and `gh pr edit --body-file -` blocks forever on a stdin that never closes.
    p = subprocess.run(["gh", *args], cwd=repo, input=stdin, capture_output=True,
                       text=True, encoding="utf-8")
    if p.returncode != 0:
        sys.exit(f"gh {' '.join(args)} failed: {p.stderr.strip() or p.stdout.strip()}")
    return p.stdout


def splice(body: str, report: str) -> str:
    """Replace the marked region, so re-running updates in place instead of stacking tables.

    Bodies with no markers (a PR opened before this script existed) get the section appended
    under its own heading; bodies from the PR template already carry the markers.
    """
    body = body.replace("\r\n", "\n")
    marked = wrap(report)
    i, j = body.find(START), body.find(END)
    if i != -1 and j != -1 and j > i:
        return body[:i] + marked + body[j + len(END):]
    return body.rstrip() + f"\n\n{HEADING}\n\n{marked}\n"


def update_pr(repo: Path, branch: str, report: str, number: str | None, dry_run: bool) -> None:
    if number is None:
        found = json.loads(gh(
            ["pr", "list", "--head", branch, "--state", "open", "--json", "number", "--limit", "1"],
            repo,
        ) or "[]")
        if not found:
            sys.exit(
                f"No open PR found for branch {branch!r}. Open the PR first, then re-run with --pr."
            )
        number = str(found[0]["number"])

    body = json.loads(gh(["pr", "view", number, "--json", "body"], repo))["body"] or ""
    updated = splice(body, report)
    if updated.strip() == body.replace("\r\n", "\n").strip():
        print(f"PR #{number} already up to date.", file=sys.stderr)
        return
    if dry_run:
        print(updated)
        print(f"\n--- dry run: PR #{number} not modified ---", file=sys.stderr)
        return
    gh(["pr", "edit", number, "--body-file", "-"], repo, stdin=updated)
    print(f"Token usage table written to PR #{number}.", file=sys.stderr)


def as_json(branch: str, mine: list[Record], strict: bool) -> str:
    by_model, _, sessions = summarize(mine)
    payload = {
        "branch": branch,
        "mode": "strict" if strict else "claim",
        "totalCalls": len(mine),
        "totalCostUSD": round(sum(b.cost(m) or 0.0 for m, b in by_model.items()), 4),
        "sessions": [
            {
                "id": sid,
                "origin": s.origin,
                "first": s.first,
                "last": s.last,
                "calls": s.calls,
                "tokens": s.tokens,
                "agents": sorted(s.agents),
            }
            for sid, s in sorted(sessions.items(), key=lambda kv: kv[1].first)
        ],
        "models": {
            m: {
                "calls": b.calls, "input": b.input, "cacheWrite5m": b.w5, "cacheWrite1h": b.w1h,
                "cacheRead": b.read, "output": b.output, "thinking": b.think,
                "costUSD": round(b.cost(m), 4) if b.cost(m) is not None else None,
            }
            for m, b in sorted(by_model.items(), key=lambda kv: -kv[1].output)
        },
    }
    return json.dumps(payload, indent=2)


def main() -> None:
    ap = argparse.ArgumentParser(description="Report Claude Code token usage for a git branch.")
    ap.add_argument("branch", nargs="?", help="branch to report on (default: current branch)")
    ap.add_argument("--repo", default=".", help="repository path (default: cwd)")
    ap.add_argument("--strict", action="store_true",
                    help="naive gitBranch filter: no session claim, no worktree recovery")
    ap.add_argument("--json", action="store_true", help="emit JSON instead of markdown")
    ap.add_argument("--pr", nargs="?", const="", metavar="N",
                    help="splice the table into PR N (default: this branch's open PR)")
    ap.add_argument("--dry-run", action="store_true",
                    help="with --pr, print the body without editing")
    ap.add_argument("--no-cost", action="store_true", help="omit the estimated-cost column")
    args = ap.parse_args()

    # The markdown uses arrows, middots and em dashes; Windows consoles default to cp1252. Force
    # LF too, so the body handed to `gh pr edit --body-file -` is byte-identical across platforms.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", newline="\n")

    cwd = Path(args.repo).resolve()
    branch = args.branch or subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=cwd, capture_output=True, text=True, check=True,
    ).stdout.strip()

    # Run from a worktree and `.git` is a file pointing into the main checkout. Resolve to that
    # checkout: it owns the worktree list, and its slug is the prefix every root shares.
    common = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=cwd, capture_output=True, text=True,
    )
    repo = Path(common.stdout.strip()).resolve().parent if common.returncode == 0 and \
        common.stdout.strip() else cwd

    roots, main_slug = transcript_roots(repo)
    records = load_records(roots, main_slug, worktree_branches(repo), args.strict)
    mine = select(records, branch, args.strict)
    if not mine:
        sys.exit(f"No transcript records found for branch {branch!r}. "
                 "Transcripts are local and purged after cleanupPeriodDays (default 30).")

    if args.json:
        print(as_json(branch, mine, args.strict))
        return

    report = render(branch, mine, args.strict, show_cost=not args.no_cost)
    if args.pr is None:
        print(wrap(report))
    else:
        update_pr(repo, branch, report, args.pr or None, args.dry_run)


if __name__ == "__main__":
    main()
