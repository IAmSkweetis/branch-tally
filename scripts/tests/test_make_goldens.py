"""Tests for scripts/make_goldens.py: pure pieces, plus end-to-end runs of the real reference."""

from __future__ import annotations

import contextlib
import io
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import make_goldens as mg  # noqa: E402


def jl(path: Path, recs) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(r if isinstance(r, str) else json.dumps(r) for r in recs) + "\n",
                    encoding="utf-8", newline="\n")


def rec(rid, branch="feat/x", session="s1", model="claude-test-1", ts="2026-01-01T00:00:00.000Z",
        **extra):
    d = {"type": "assistant", "requestId": rid, "uuid": "u-" + rid, "sessionId": session,
         "timestamp": ts, "message": {"model": model, "usage": {
             "input_tokens": 10, "output_tokens": 20, "cache_read_input_tokens": 30}}}
    if branch is not None:
        d["gitBranch"] = branch
    d.update(extra)
    return d


class Pure(unittest.TestCase):
    def test_substitute_slug(self):
        s = mg.substitute_slug
        self.assertEqual(s("C--repo", "C--repo", "C--tmp-x-repo"), "C--tmp-x-repo")
        self.assertEqual(s("C--repo--claude-worktrees-a", "C--repo", "C--tmp-x-repo"),
                         "C--tmp-x-repo--claude-worktrees-a")
        self.assertEqual(s("C--repo-cardgame", "C--repo", "C--tmp-x-repo"),
                         "C--tmp-x-repo-cardgame")
        # a case difference from repo_slug survives as a case difference (lowercased prefix)
        self.assertEqual(s("c--repo", "C--repo", "C--Tmp-x-repo"), "c--tmp-x-repo")
        self.assertEqual(s("C--Repo--Claude-Worktrees-b", "C--repo", "C--tmp-x-repo"),
                         "c--tmp-x-repo--Claude-Worktrees-b")
        self.assertIsNone(s("C--other", "C--repo", "C--tmp-x-repo"))

    def test_plan_copies_and_origin_map(self):
        plan = mg.plan_copies(["C--repo", "C--other", "C--repo--claude-worktrees-a"],
                              "C--repo", "C--T-repo")
        self.assertEqual(plan, {"C--repo": "C--T-repo", "C--other": "C--other",
                                "C--repo--claude-worktrees-a": "C--T-repo--claude-worktrees-a"})
        self.assertEqual(mg.origin_map(plan), {
            "c--t-repo": "C--repo", "c--other": "C--other",
            "c--t-repo--claude-worktrees-a": "C--repo--claude-worktrees-a"})
        with self.assertRaises(mg.GoldenError):
            mg.origin_map({"x": "A", "y": "a"})

    def test_normalize_origin(self):
        payload = {"branch": "b", "sessions": [
            {"id": "s", "origin": "main"},
            {"id": "t", "origin": "c--t-repo--claude-worktrees-a"}]}
        out = mg.normalize_payload(
            payload, {"c--t-repo--claude-worktrees-a": "C--repo--claude-worktrees-a"})
        self.assertEqual([s["origin"] for s in out["sessions"]],
                         ["main", "C--repo--claude-worktrees-a"])
        with self.assertRaises(mg.GoldenError):
            mg.normalize_payload(payload, {})

    def test_parse_result(self):
        self.assertEqual(mg.parse_result(0, '{"a": 1}', ""), {"a": 1})
        no = mg.parse_result(1, "", "No transcript records found for branch 'x'. Transcripts...")
        self.assertEqual(no, {"error": "no_records"})
        with self.assertRaises(mg.GoldenError):
            mg.parse_result(1, "", "Traceback boom")
        with self.assertRaises(mg.GoldenError):
            mg.parse_result(0, "not json", "")

    def test_enumerate_branches(self):
        with tempfile.TemporaryDirectory() as t:
            r = Path(t)
            jl(r / "C--repo" / "a.jsonl", [
                rec("1", "feat/x"), rec("2", None), rec("3", ""), rec("4", "main"),
                rec("5", "synthetic-only", model="<synthetic>"), "garbage {",
                {"type": "user", "gitBranch": "no-usage"},
            ])
            jl(r / "C--repo" / "a" / "subagents" / "agent-1.jsonl",
               [rec("7", "worktree-agent-1")])
            jl(r / "C--repo--claude-worktrees-w" / "b.jsonl", [rec("8", "feat/w")])
            jl(r / "C--repo-cardgame" / "c.jsonl", [rec("9", "sibling-only")])
            jl(r / "C--other" / "d.jsonl", [rec("10", "other-only")])
            self.assertEqual(
                mg.enumerate_branches(r, "C--repo"),
                sorted(["(none)", "feat/w", "feat/x", "main", "worktree-agent-1"]))


@unittest.skipUnless(shutil.which("git"), "needs git")
class EndToEnd(unittest.TestCase):
    def make(self, root: Path) -> Path:
        sc = root / "tiny"
        jl(sc / "C--repo" / "s1.jsonl", [
            rec("a", "feat/x"), rec("b", None, ts="2026-01-01T00:01:00.000Z"),
            rec("c", "feat/x", session="s2", ts="2026-01-02T00:00:00.000Z")])
        jl(sc / "C--repo--claude-worktrees-w1" / "s3.jsonl",
           [rec("d", "worktree-w1", session="s3", ts="2026-01-03T00:00:00.000Z")])
        jl(sc / "C--repo-cardgame" / "s9.jsonl", [rec("z", "feat/x", session="s9")])
        (sc / "goldens").mkdir()
        (sc / "goldens" / "scenario.json").write_text(json.dumps({
            "repo_slug": "C--repo",
            "worktrees": [{"name": "w1", "branch": "feat/w1"}],
            "claim_branches": ["feat/x", "nonexistent"]}), encoding="utf-8")
        return sc

    def test_generates_deterministic_normalized_goldens(self):
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            sc = self.make(root)
            (root / "nomanifest").mkdir()
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                self.assertEqual(mg.main(["--root", str(root)]), 0)
            self.assertIn("nomanifest", err.getvalue())
            strict_p, claim_p = sc / "goldens" / "strict.json", sc / "goldens" / "claim.json"
            first = (strict_p.read_bytes(), claim_p.read_bytes())
            self.assertTrue(first[0].endswith(b"\n") and b"\r" not in first[0])
            strict = json.loads(first[0])
            self.assertEqual(sorted(strict), ["(none)", "feat/x", "worktree-w1"])
            self.assertEqual(strict["feat/x"]["totalCalls"], 2)
            self.assertEqual(strict["feat/x"]["mode"], "strict")
            self.assertEqual(strict["(none)"]["totalCalls"], 1)
            self.assertEqual(strict["worktree-w1"]["sessions"][0]["origin"],
                             "C--repo--claude-worktrees-w1")
            self.assertEqual(strict["feat/x"]["sessions"][0]["origin"], "main")
            claim = json.loads(first[1])
            self.assertEqual(claim["feat/x"]["mode"], "claim")
            self.assertEqual(claim["nonexistent"], {"error": "no_records"})
            text = first[0].decode() + first[1].decode()
            for needle in (t, Path(t).name, "Users", "AppData"):
                self.assertNotIn(needle, text)
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(mg.main(["--root", str(root), "tiny"]), 0)
                self.assertEqual(mg.main(["--root", str(root), "--check"]), 0)
            self.assertEqual(first, (strict_p.read_bytes(), claim_p.read_bytes()))
            strict_p.write_text("{}\n", encoding="utf-8")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(mg.main(["--root", str(root), "--check"]), 1)

    def test_unknown_filter_fails(self):
        with tempfile.TemporaryDirectory() as t:
            self.make(Path(t))
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(mg.main(["--root", t, "missing"]), 1)


if __name__ == "__main__":
    unittest.main()
