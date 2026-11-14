"""Tests for scripts/repo_settings.py against a fake `gh` on PATH (scripts/tests/fake_gh.py).

The fake serves canned GETs and applies PUT/POST/DELETE to a state file, and every call is logged,
so these tests assert the exact API calls for apply, idempotence and --check drift detection.
Nothing here talks to GitHub.
"""

from __future__ import annotations

import copy
import io
import json
import os
import re
import shutil
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

import repo_settings as rs  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
FAKE = HERE / "fake_gh.py"
NAME = "octo/widgets"

# What the live repository looked like on 2026-11-14, before anything is applied.
LIVE = {
    "repo": NAME,
    "users": {"IAmSkweetis": 174960678},
    "immutable": False,
    "rulesets": [{"id": 1, "name": "default", "target": "branch", "enforcement": "active",
                  "bypass_actors": [], "conditions": {}, "rules": [{"type": "pull_request"}]}],
    "environment": None,
    "actions": {
        "permissions": {"enabled": True, "allowed_actions": "all", "sha_pinning_required": False},
        "selected": {},
        "fork_approval": "first_time_contributors",
    },
}

WANT_MUTATIONS = [
    ("PUT", "immutable-releases"),
    ("POST", "rulesets"),
    ("PUT", "environments/release"),
    ("POST", "environments/release/deployment-branch-policies"),
    ("PUT", "actions/permissions"),
    ("PUT", "actions/permissions/selected-actions"),
    ("PUT", "actions/permissions/fork-pr-contributor-approval"),
]


class Harness(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.tmp, ignore_errors=True))
        self.bindir = self.tmp / "bin"
        self.bindir.mkdir()
        if os.name == "nt":
            (self.bindir / "gh.cmd").write_text(
                f'@"{sys.executable}" "{FAKE}" %*\r\n', encoding="utf-8")
        else:
            gh = self.bindir / "gh"
            gh.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{FAKE}" "$@"\n', encoding="utf-8")
            gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
        self.state = self.tmp / "state.json"
        self.log = self.tmp / "log.jsonl"
        self.set_state(LIVE)
        env = {"PATH": str(self.bindir) + os.pathsep + os.environ["PATH"],
               "FAKE_GH_STATE": str(self.state), "FAKE_GH_LOG": str(self.log)}
        patcher = mock.patch.dict(os.environ, env)
        patcher.start()
        self.addCleanup(patcher.stop)

    def set_state(self, st):
        self.state.write_text(json.dumps(st), encoding="utf-8")

    def get_state(self):
        return json.loads(self.state.read_text(encoding="utf-8"))

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(x) for x in self.log.read_text(encoding="utf-8").splitlines()]

    def mutations(self):
        pre = f"repos/{NAME}/"
        return [(c["method"], c["path"][len(pre):]) for c in self.calls() if c["method"] != "GET"]

    def reset_log(self):
        self.log.unlink(missing_ok=True)

    def run_script(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = rs.main(list(args))
        return code, out.getvalue(), err.getvalue()


class Apply(Harness):
    def test_apply_makes_exactly_the_expected_calls_in_order(self):
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        self.assertEqual(self.mutations(), WANT_MUTATIONS)

    def test_apply_bodies(self):
        self.run_script()
        pre = f"repos/{NAME}/"
        by = {(c["method"], c["path"][len(pre):]): c["body"]
              for c in self.calls() if c["method"] != "GET"}
        rules = by[("POST", "rulesets")]
        self.assertEqual(rules["target"], "tag")
        self.assertEqual(rules["enforcement"], "active")
        self.assertEqual(rules["bypass_actors"], [])
        self.assertEqual(rules["conditions"]["ref_name"]["include"], ["refs/tags/v*"])
        self.assertEqual(sorted(r["type"] for r in rules["rules"]), ["deletion", "update"])
        env = by[("PUT", "environments/release")]
        self.assertEqual(env["reviewers"], [{"type": "User", "id": 174960678}])
        self.assertIs(env["prevent_self_review"], False)
        self.assertIs(env["can_admins_bypass"], False)
        self.assertEqual(env["deployment_branch_policy"],
                         {"protected_branches": False, "custom_branch_policies": True})
        self.assertEqual(by[("POST", "environments/release/deployment-branch-policies")],
                         {"name": "main", "type": "branch"})
        self.assertEqual(by[("PUT", "actions/permissions")],
                         {"enabled": True, "allowed_actions": "selected",
                          "sha_pinning_required": True})
        sel = by[("PUT", "actions/permissions/selected-actions")]
        self.assertIs(sel["github_owned_allowed"], True)
        self.assertEqual(sorted(sel["patterns_allowed"]), sorted(rs.PATTERNS_ALLOWED))
        self.assertEqual(by[("PUT", "actions/permissions/fork-pr-contributor-approval")],
                         {"approval_policy": "all_external_contributors"})

    def test_apply_leaves_the_other_ruleset_alone(self):
        self.run_script()
        names = [r["name"] for r in self.get_state()["rulesets"]]
        self.assertEqual(names, ["default", rs.RULESET_NAME])

    def test_second_run_makes_no_mutating_call(self):
        self.run_script()
        self.reset_log()
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        self.assertEqual(self.mutations(), [])
        self.assertTrue(self.calls(), "a rerun must still read the live settings")

    def test_existing_ruleset_of_the_same_name_is_updated_not_duplicated(self):
        st = copy.deepcopy(LIVE)
        st["rulesets"].append({"id": 7, "name": rs.RULESET_NAME, "target": "tag",
                               "enforcement": "active", "bypass_actors": [],
                               "conditions": {"ref_name": {"include": ["refs/tags/v*"],
                                                           "exclude": []}},
                               "rules": [{"type": "update"}]})  # deletion missing
        self.set_state(st)
        self.run_script()
        muts = self.mutations()
        self.assertIn(("PUT", "rulesets/7"), muts)
        self.assertNotIn(("POST", "rulesets"), muts)
        names = [r["name"] for r in self.get_state()["rulesets"]]
        self.assertEqual(names.count(rs.RULESET_NAME), 1)

    def test_repo_comes_from_gh_repo_view(self):
        self.run_script()
        self.assertTrue(all(c["path"].startswith(f"repos/{NAME}/") or c["path"].startswith("users/")
                            for c in self.calls()))

    def test_repo_flag_overrides_gh_repo_view(self):
        st = copy.deepcopy(LIVE)
        st["repo"] = "other/thing"
        self.set_state(st)
        code, _, err = self.run_script("--repo", "other/thing", "--check")
        self.assertEqual(code, 1, err)
        self.assertTrue(any(c["path"].startswith("repos/other/thing/") for c in self.calls()))

    def test_unknown_reviewer_fails_without_mutating(self):
        code, _, err = self.run_script("--reviewer", "nobody")
        self.assertNotEqual(code, 0)
        self.assertIn("nobody", err)
        self.assertEqual(self.mutations(), [])

    def test_missing_gh_fails_loudly(self):
        with mock.patch.dict(os.environ, {"PATH": str(self.tmp / "empty")}):
            code, _, err = self.run_script("--check")
        self.assertNotEqual(code, 0)
        self.assertIn("gh", err)


class Check(Harness):
    def test_check_on_the_live_state_lists_every_drift_and_never_mutates(self):
        code, out, err = self.run_script("--check")
        self.assertEqual(code, 1, out + err)
        for word in ("immutable", "ruleset", "environment", "sha pinning", "allowed actions",
                     "fork"):
            self.assertIn(word, out.lower(), f"drift report omits {word!r}:\n{out}")
        self.assertEqual(self.mutations(), [])

    def test_check_passes_after_apply(self):
        self.run_script()
        self.reset_log()
        code, out, err = self.run_script("--check")
        self.assertEqual(code, 0, out + err)
        self.assertEqual(self.mutations(), [])

    def applied(self):
        self.run_script()
        self.reset_log()
        return self.get_state()

    def drift(self, mutate, word):
        st = self.applied()
        mutate(st)
        self.set_state(st)
        code, out, err = self.run_script("--check")
        self.assertEqual(code, 1, out + err)
        self.assertIn(word, out.lower())
        self.assertEqual(self.mutations(), [])

    def tag_ruleset(self, st):
        return next(r for r in st["rulesets"] if r["name"] == rs.RULESET_NAME)

    def test_drift_immutable_off(self):
        self.drift(lambda s: s.update(immutable=False), "immutable")

    def test_drift_ruleset_gained_a_bypass_actor(self):
        def m(s):
            self.tag_ruleset(s)["bypass_actors"] = [
                {"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always"}]
        self.drift(m, "bypass")

    def test_drift_ruleset_lost_deletion_rule(self):
        self.drift(lambda s: self.tag_ruleset(s).update(rules=[{"type": "update"}]), "ruleset")

    def test_drift_ruleset_blocks_creation(self):
        self.drift(lambda s: self.tag_ruleset(s)["rules"].append({"type": "creation"}), "ruleset")

    def test_drift_ruleset_disabled(self):
        self.drift(lambda s: self.tag_ruleset(s).update(enforcement="disabled"), "ruleset")

    def test_drift_admin_bypass_on(self):
        self.drift(lambda s: s["environment"].update(can_admins_bypass=True), "admin")

    def test_drift_self_review_prevented(self):
        self.drift(lambda s: s["environment"].update(prevent_self_review=True), "self")

    def test_drift_reviewer_changed(self):
        self.drift(lambda s: s["environment"].update(reviewers=[{"type": "User", "id": 9}]),
                   "reviewer")

    def test_drift_extra_deployment_branch(self):
        def m(s):
            s["environment"]["policies"].append({"id": 900, "name": "*", "type": "branch"})
        self.drift(m, "deployment")

    def test_drift_pinning_off(self):
        def m(s):
            s["actions"]["permissions"]["sha_pinning_required"] = False
        self.drift(m, "sha pinning")

    def test_drift_extra_allowed_pattern(self):
        def m(s):
            s["actions"]["selected"]["patterns_allowed"].append("evil/action@*")
        self.drift(m, "allowed actions")

    def test_drift_missing_allowed_pattern(self):
        def m(s):
            s["actions"]["selected"]["patterns_allowed"].pop()
        self.drift(m, "allowed actions")

    def test_drift_fork_approval(self):
        def m(s):
            s["actions"]["fork_approval"] = "first_time_contributors"
        self.drift(m, "fork")


class AllowedActions(unittest.TestCase):
    """The allow-list must cover exactly the non-GitHub actions the workflows use."""

    def used(self):
        found = set()
        for wf in (REPO / ".github" / "workflows").glob("*.yml"):
            for line in wf.read_text(encoding="utf-8").splitlines():
                m = re.match(r"\s*(?:-\s*)?uses:\s*([^\s#]+)", line)
                if not m:
                    continue
                ref = m.group(1)
                if ref.startswith(("./", "$/")):
                    continue  # local reusable workflow
                owner_repo = ref.split("@")[0]
                owner = owner_repo.split("/")[0]
                if owner not in ("actions", "github"):
                    found.add(owner_repo)
        return found

    def test_every_third_party_action_is_allowed_and_nothing_else(self):
        used = self.used()
        self.assertTrue(used, "found no third-party actions: the scan is broken")
        allowed = {p.split("@")[0] for p in rs.PATTERNS_ALLOWED}
        self.assertEqual(used, allowed)

    def test_patterns_name_a_repo_with_any_ref(self):
        self.assertTrue(rs.PATTERNS_ALLOWED)
        for p in rs.PATTERNS_ALLOWED:
            self.assertRegex(p, r"^[\w.-]+/[\w.-]+@\*$")


def job_block(text: str, job: str) -> str:
    m = re.search(rf"^  {job}:\n(.*?)(?=^  \w[\w-]*:\n|\Z)", text, re.S | re.M)
    assert m, f"no job {job}"
    return m.group(0)


class Workflows(unittest.TestCase):
    def setUp(self):
        wf = REPO / ".github" / "workflows"
        self.release = (wf / "release.yml").read_text(encoding="utf-8")
        self.build = (wf / "build.yml").read_text(encoding="utf-8")

    def test_publish_job_runs_in_the_release_environment(self):
        blk = job_block(self.release, "publish")
        self.assertRegex(blk, r"(?m)^    environment:")
        self.assertRegex(blk, r"name:\s*release\b")

    def test_no_other_job_uses_the_environment_or_the_signing_secrets(self):
        for job in ("version", "release", "build"):
            blk = job_block(self.release, job)
            self.assertNotIn("environment:", blk)
            self.assertNotIn("MINISIGN", blk)

    def test_release_passes_the_vetted_sha_to_build(self):
        self.assertRegex(job_block(self.release, "build"),
                         r"sha:\s*\$\{\{\s*needs\.version\.outputs\.sha\s*\}\}")

    def test_build_takes_a_sha_input_and_checks_it_out(self):
        self.assertRegex(self.build, r"(?m)^      sha:\n(?:        .*\n)*?        required: true")
        job = job_block(self.build, "build")
        self.assertRegex(job, r"ref:\s*\$\{\{\s*inputs\.sha\s*\}\}")
        self.assertNotRegex(job, r"ref:\s*\$\{\{\s*inputs\.version\s*\}\}")

    def test_build_fails_when_the_tag_no_longer_resolves_to_the_sha(self):
        job = job_block(self.build, "build")
        self.assertIn("rev-parse", job)
        self.assertIn("^{commit}", job)

    def test_no_expression_inside_run(self):
        for text in (self.release, self.build):
            for m in re.finditer(r"(?m)^\s*run:\s*(.*)$", text):
                self.assertNotIn("${{", m.group(1))


if __name__ == "__main__":
    unittest.main()
