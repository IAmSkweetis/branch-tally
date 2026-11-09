"""Tests for scripts/scrub_transcripts.py. Inputs are synthetic and built in temp dirs."""

from __future__ import annotations

import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import scrub_transcripts as sc  # noqa: E402

SECRET = "PRIVATE-PROSE-must-not-leak"
ROOT_WIN = "D:\\Users\\bob\\proj"


def record(**over):
    base = {
        "type": "assistant",
        "requestId": "req_real_1",
        "uuid": "uuid-real-1",
        "sessionId": "sess-real-1",
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": ROOT_WIN,
        "gitBranch": "feat/x",
        "isSidechain": False,
        "userType": SECRET,
        "message": {
            "model": "claude-test-1",
            "content": [{"type": "text", "text": SECRET}],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_read_input_tokens": 30,
                "cache_creation_input_tokens": 4,
                "cache_creation": {"ephemeral_5m_input_tokens": 1, "ephemeral_1h_input_tokens": 3},
                "output_tokens_details": {"thinking_tokens": 7},
                "service_tier": SECRET,
            },
        },
        "toolUseResult": SECRET,
    }
    base.update(over)
    return base


def write_jsonl(path: Path, lines) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    out = [line if isinstance(line, str) else json.dumps(line) for line in lines]
    path.write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")


def tree_text(root: Path) -> str:
    parts = []
    for p in sorted(root.rglob("*")):
        parts.append(str(p.relative_to(root)))
        if p.is_file():
            parts.append(p.read_text(encoding="utf-8"))
    return "\n".join(parts)


def run_scrub(src: Path, dest: Path, root: str = ROOT_WIN):
    err = io.StringIO()
    with contextlib.redirect_stderr(err):
        stats = sc.scrub_tree(src, dest, root)
    return stats, err.getvalue()


class RewriteCwd(unittest.TestCase):
    def test_forms(self):
        n = sc.normalize_root(ROOT_WIN)
        cases = {
            ROOT_WIN: "C:\\repo",
            ROOT_WIN + "\\.claude\\worktrees\\a1": "C:\\repo\\.claude\\worktrees\\a1",
            "/d/Users/bob/proj": "/repo",
            "/d/Users/bob/proj/.claude/worktrees/a1": "/repo/.claude/worktrees/a1",
            "d:\\users\\BOB\\proj\\sub": "C:\\repo\\sub",
            "D:\\Users\\bob\\proj-other": "C:\\other",
            "/home/alice/else": "/other",
        }
        for given, want in cases.items():
            self.assertEqual(sc.rewrite_cwd(given, n), want, given)


class ScrubTree(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.tmp = Path(self._t.name)
        self.src = self.tmp / "src"
        self.dest = self.tmp / "dest"
        self.slug = self.src / "D--Users-bob-proj"

    def test_content_and_unknown_fields_are_gone(self):
        write_jsonl(self.slug / "sess-real-1.jsonl", [record()])
        stats, err = run_scrub(self.src, self.dest)
        text = tree_text(self.dest)
        for needle in (SECRET, "bob", "Users", "toolUseResult", "userType", "service_tier"):
            self.assertNotIn(needle, text)
            self.assertNotIn(needle, err)
        self.assertEqual(stats.records_written, 1)
        out = json.loads(next(self.dest.rglob("*.jsonl")).read_text(encoding="utf-8"))
        self.assertEqual(out["message"]["content"], "PLACEHOLDER")
        self.assertEqual(out["cwd"], "C:\\repo")
        self.assertEqual(out["message"]["model"], "claude-test-1")
        self.assertEqual(out["message"]["usage"], {
            "input_tokens": 10, "output_tokens": 20, "cache_read_input_tokens": 30,
            "cache_creation_input_tokens": 4,
            "cache_creation": {"ephemeral_5m_input_tokens": 1, "ephemeral_1h_input_tokens": 3},
            "output_tokens_details": {"thinking_tokens": 7},
        })

    def test_ids_remapped_equality_preserved_and_paths_renamed(self):
        a = record()
        b = record(uuid="uuid-real-2")  # same requestId: a resumed-history replay
        c = record(requestId="req_real_2", uuid="uuid-real-3")
        write_jsonl(self.slug / "sess-real-1.jsonl", [a, b, c])
        sub = record(sessionId="sess-real-1", agentId="agent-real", isSidechain=True,
                     requestId="req_real_3", uuid="uuid-real-4")
        write_jsonl(self.slug / "sess-real-1" / "subagents" / "agent-agent-real.jsonl", [sub])
        run_scrub(self.src, self.dest)
        text = tree_text(self.dest)
        for real in ("real", "sess-real", "agent-real"):
            self.assertNotIn(real, text)
        main = [json.loads(line) for line in
                (self.dest / "C--repo" / "s1.jsonl").read_text(encoding="utf-8").splitlines()]
        self.assertEqual(main[0]["requestId"], main[1]["requestId"])
        self.assertNotEqual(main[0]["requestId"], main[2]["requestId"])
        self.assertNotEqual(main[0]["uuid"], main[1]["uuid"])
        agent_file = self.dest / "C--repo" / "s1" / "subagents" / "agent-a1.jsonl"
        s = json.loads(agent_file.read_text(encoding="utf-8"))
        self.assertEqual(s["sessionId"], main[0]["sessionId"])
        self.assertEqual(s["agentId"], "a1")

    def test_malformed_lines_are_counted_never_echoed(self):
        write_jsonl(self.slug / "s.jsonl", [record(), "{" + SECRET, "not json " + SECRET, ""])
        stats, err = run_scrub(self.src, self.dest)
        self.assertEqual(stats.malformed_lines, 2)
        self.assertEqual(stats.records_written, 1)
        self.assertNotIn(SECRET, err)
        self.assertNotIn(SECRET, tree_text(self.dest))
        self.assertIn("2", err)

    def test_bad_typed_usage_drops_the_line_and_counts_it(self):
        bad = record()
        bad["message"]["usage"]["input_tokens"] = SECRET
        write_jsonl(self.slug / "s.jsonl", [bad])
        stats, err = run_scrub(self.src, self.dest)
        self.assertEqual(stats.records_written, 0)
        self.assertEqual(stats.unscrubbable_lines, 1)
        self.assertNotIn(SECRET, err)

    def test_deterministic(self):
        write_jsonl(self.slug / "s.jsonl", [record(), record(requestId="r2", uuid="u2")])
        run_scrub(self.src, self.dest)
        again = self.tmp / "dest2"
        run_scrub(self.src, again)
        self.assertEqual(tree_text(self.dest), tree_text(again))

    def test_refuses_nonempty_destination(self):
        write_jsonl(self.slug / "s.jsonl", [record()])
        self.dest.mkdir()
        (self.dest / "keep.txt").write_text("x", encoding="utf-8")
        with self.assertRaises(sc.ScrubError):
            run_scrub(self.src, self.dest)

    def test_slug_dir_as_source(self):
        write_jsonl(self.slug / "s.jsonl", [record()])
        run_scrub(self.slug, self.dest)
        self.assertTrue((self.dest / "C--repo" / "s1.jsonl").is_file())


class SelfCheck(unittest.TestCase):
    def test_flags_a_surviving_string_and_passes_a_clean_tree(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            root = sc.normalize_root(ROOT_WIN)
            clean = {"type": "assistant", "requestId": "req_1", "message": {
                "model": "m", "content": "PLACEHOLDER", "usage": {"input_tokens": 1}}}
            write_jsonl(d / "C--repo" / "s1.jsonl", [clean])
            self.assertEqual(sc.self_check(d, root), [])
            write_jsonl(d / "C--repo" / "s2.jsonl", [dict(clean, prompt=SECRET)])
            problems = sc.self_check(d, root)
            self.assertTrue(problems)
            self.assertNotIn(SECRET, " ".join(problems))
            write_jsonl(d / "C--repo" / "s2.jsonl", [dict(clean, cwd="D:\\Users\\bob\\proj\\x")])
            self.assertTrue(sc.self_check(d, root))


if __name__ == "__main__":
    unittest.main()
