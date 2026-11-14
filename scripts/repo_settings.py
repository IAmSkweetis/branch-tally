"""Apply, or check, the repository settings the release pipeline's safety depends on (#97, F-05).

    uv run scripts/repo_settings.py            # apply: change whatever differs, then re-check
    uv run scripts/repo_settings.py --check    # read-only: list every drift, exit 1 if any

`just repo-settings` runs it. It needs the `gh` CLI, logged in as a repository admin.

The desired state:

* immutable releases are on (a published release's assets and tag can no longer change);
* a tag ruleset, RULESET_NAME, blocks updating and deleting `v*` tags, with no bypass actors.
  Creation stays allowed, because release.yml creates the tag with GITHUB_TOKEN;
* the `release` environment, which release.yml's publish job runs in, has the maintainer as its
  required reviewer (self-review allowed: there is one maintainer), no admin bypass, and may be
  deployed from `main` only;
* Actions may use only GitHub-owned actions plus PATTERNS_ALLOWED, and every `uses:` must be a full
  commit SHA;
* workflow runs from all outside contributors need approval.

Apply mode is idempotent: it GETs every setting and calls PUT/POST/DELETE only for a setting that
differs, so a rerun changes nothing. --check never calls anything but GET. Every setting is read
through `gh api`, so nothing here handles a token. The repository is the one `gh repo view` reports
for the current directory, unless --repo says otherwise.

PATTERNS_ALLOWED is the list of non-GitHub actions .github/workflows/*.yml uses. A test
(scripts/tests/test_repo_settings.py) fails when a workflow starts using an action that is not on
it, so add the action here in the same change, then re-run this script.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from dataclasses import dataclass, field

DEFAULT_REVIEWER = "IAmSkweetis"
RULESET_NAME = "release-tags"
ENVIRONMENT = "release"
DEPLOY_BRANCH = "main"
TAG_PATTERN = "refs/tags/v*"

# Every non-GitHub action the workflows use. `@*` because they are pinned by commit SHA.
PATTERNS_ALLOWED = [
    "astral-sh/setup-uv@*",
    "dorny/paths-filter@*",
    "sigstore/cosign-installer@*",
    "taiki-e/install-action@*",
]


class GhError(Exception):
    def __init__(self, message: str, status: int | None = None):
        super().__init__(message)
        self.status = status


@dataclass
class Step:
    """One mutating API call, with the drift it fixes."""

    drift: str
    method: str
    path: str  # relative to repos/{repo}/
    body: dict | None = None


@dataclass
class Plan:
    drifts: list[str] = field(default_factory=list)
    steps: list[Step] = field(default_factory=list)

    def add(self, step: Step) -> None:
        """Record a drift. Two drifts fixed by the same call share one call."""
        if step.drift not in self.drifts:
            self.drifts.append(step.drift)
        if not any((s.method, s.path, s.body) == (step.method, step.path, step.body)
                   for s in self.steps):
            self.steps.append(step)


def gh(args: list[str], body: dict | None = None) -> str:
    exe = shutil.which("gh")
    if exe is None:
        raise GhError("the gh CLI is not on PATH; install it: https://cli.github.com/")
    cmd = [exe, *args]
    if body is not None:
        cmd += ["--input", "-"]
    proc = subprocess.run(cmd, input=None if body is None else json.dumps(body),
                          capture_output=True, text=True, encoding="utf-8")
    if proc.returncode != 0:
        err = (proc.stderr + proc.stdout).strip()
        status = None
        for code in (404, 409, 403, 401):
            if f"HTTP {code}" in err or f"({code})" in err or f'"status":"{code}"' in err:
                status = code
                break
        raise GhError(f"gh {' '.join(args)} failed: {err}", status)
    return proc.stdout


def get(repo: str, path: str):
    out = gh(["api", f"repos/{repo}/{path}"])
    return json.loads(out) if out.strip() else None


def immutable_releases(repo: str, plan: Plan) -> None:
    cur = get(repo, "immutable-releases")
    if not cur.get("enabled"):
        plan.add(Step("immutable releases: off, want on", "PUT", "immutable-releases"))


def desired_ruleset() -> dict:
    return {
        "name": RULESET_NAME,
        "target": "tag",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": {"ref_name": {"include": [TAG_PATTERN], "exclude": []}},
        "rules": [{"type": "update"}, {"type": "deletion"}],
    }


def ruleset_problems(cur: dict) -> list[str]:
    """What differs between a live tag ruleset and the desired one."""
    want = desired_ruleset()
    out = []
    if cur.get("target") != "tag":
        out.append(f"target is {cur.get('target')!r}, want 'tag'")
    if cur.get("enforcement") != "active":
        out.append(f"enforcement is {cur.get('enforcement')!r}, want 'active'")
    if cur.get("bypass_actors"):
        out.append("it has bypass actors, want none")
    ref = (cur.get("conditions") or {}).get("ref_name") or {}
    if sorted(ref.get("include") or []) != want["conditions"]["ref_name"]["include"]:
        out.append(f"it includes {ref.get('include')!r}, want [{TAG_PATTERN!r}]")
    if sorted(ref.get("exclude") or []) != []:
        out.append(f"it excludes {ref.get('exclude')!r}, want nothing")
    types = sorted(r.get("type") for r in cur.get("rules") or [])
    if types != ["deletion", "update"]:
        out.append(f"its rules are {types}, want ['deletion', 'update'] (creation must stay allowed)")
    return out


def tag_ruleset(repo: str, plan: Plan) -> None:
    listing = get(repo, "rulesets?per_page=100") or []
    same = [r for r in listing if r.get("name") == RULESET_NAME]
    if len(same) > 1:
        raise GhError(f"{len(same)} rulesets are named {RULESET_NAME!r}; delete the extras by hand")
    if not same:
        plan.add(Step(f"ruleset {RULESET_NAME!r}: missing", "POST", "rulesets", desired_ruleset()))
        return
    cur = get(repo, f"rulesets/{same[0]['id']}")
    problems = ruleset_problems(cur)
    if problems:
        plan.add(Step(f"ruleset {RULESET_NAME!r}: " + "; ".join(problems), "PUT",
                      f"rulesets/{same[0]['id']}", desired_ruleset()))


def environment(repo: str, reviewer_id: int, reviewer: str, plan: Plan) -> None:
    base = f"environments/{ENVIRONMENT}"
    put = Step("", "PUT", base, {
        "reviewers": [{"type": "User", "id": reviewer_id}],
        "prevent_self_review": False,
        "can_admins_bypass": False,
        "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True},
    })
    try:
        cur = get(repo, base)
    except GhError as e:
        if e.status != 404:
            raise
        plan.add(Step(f"environment {ENVIRONMENT!r}: missing", put.method, put.path, put.body))
        plan.add(Step(f"environment {ENVIRONMENT!r}: deployment branch {DEPLOY_BRANCH!r} missing",
                      "POST", f"{base}/deployment-branch-policies",
                      {"name": DEPLOY_BRANCH, "type": "branch"}))
        return

    problems = []
    rules = [r for r in cur.get("protection_rules") or [] if r.get("type") == "required_reviewers"]
    reviewers = {(x.get("type"), (x.get("reviewer") or {}).get("id"))
                 for r in rules for x in r.get("reviewers") or []}
    if reviewers != {("User", reviewer_id)}:
        got = sorted(f"{t}:{i}" for t, i in reviewers) or "none"
        problems.append(f"required reviewer is {got}, want User:{reviewer_id} ({reviewer})")
    if any(r.get("prevent_self_review") for r in rules):
        problems.append("self-review is prevented, want it allowed (there is one maintainer)")
    if cur.get("can_admins_bypass") is not False:
        problems.append("admins can bypass the environment, want no admin bypass")
    if cur.get("deployment_branch_policy") != put.body["deployment_branch_policy"]:
        problems.append(f"deployment branch policy is {cur.get('deployment_branch_policy')}, "
                        "want custom branches only")
    for p in problems:
        plan.add(Step(f"environment {ENVIRONMENT!r}: {p}", put.method, put.path, put.body))

    pols = (get(repo, f"{base}/deployment-branch-policies") or {}).get("branch_policies") or []
    names = sorted(p.get("name") for p in pols)
    if names != [DEPLOY_BRANCH] or any(p.get("type", "branch") != "branch" for p in pols):
        drift = (f"environment {ENVIRONMENT!r}: deployment branches are {names or 'none'}, "
                 f"want only {DEPLOY_BRANCH!r}")
        for p in pols:
            if p.get("name") != DEPLOY_BRANCH or p.get("type", "branch") != "branch":
                plan.add(Step(drift, "DELETE", f"{base}/deployment-branch-policies/{p['id']}"))
        if not any(p.get("name") == DEPLOY_BRANCH and p.get("type", "branch") == "branch"
                   for p in pols):
            plan.add(Step(drift, "POST", f"{base}/deployment-branch-policies",
                          {"name": DEPLOY_BRANCH, "type": "branch"}))


def actions(repo: str, plan: Plan) -> None:
    cur = get(repo, "actions/permissions")
    want = {"enabled": True, "allowed_actions": "selected", "sha_pinning_required": True}
    if not cur.get("sha_pinning_required"):
        plan.add(Step("actions: sha pinning not required, want required", "PUT",
                      "actions/permissions", want))
    if cur.get("allowed_actions") != "selected":
        plan.add(Step(f"allowed actions: {cur.get('allowed_actions')!r}, want 'selected'", "PUT",
                      "actions/permissions", want))
        selected = None  # unreadable until the policy is 'selected' (GitHub answers 409)
    else:
        selected = get(repo, "actions/permissions/selected-actions")
    want_sel = {"github_owned_allowed": True, "verified_allowed": False,
                "patterns_allowed": sorted(PATTERNS_ALLOWED)}
    if selected is None:
        plan.add(Step("allowed actions: GitHub-owned plus the listed patterns wanted", "PUT",
                      "actions/permissions/selected-actions", want_sel))
    else:
        have = {"github_owned_allowed": bool(selected.get("github_owned_allowed")),
                "verified_allowed": bool(selected.get("verified_allowed")),
                "patterns_allowed": sorted(selected.get("patterns_allowed") or [])}
        if have != want_sel:
            extra = sorted(set(have["patterns_allowed"]) - set(want_sel["patterns_allowed"]))
            miss = sorted(set(want_sel["patterns_allowed"]) - set(have["patterns_allowed"]))
            why = []
            if extra:
                why.append(f"extra patterns {extra}")
            if miss:
                why.append(f"missing patterns {miss}")
            if have["github_owned_allowed"] is not True:
                why.append("GitHub-owned actions not allowed")
            if have["verified_allowed"] is not False:
                why.append("verified creators allowed")
            plan.add(Step("allowed actions: " + "; ".join(why), "PUT",
                          "actions/permissions/selected-actions", want_sel))
    fork = get(repo, "actions/permissions/fork-pr-contributor-approval")
    if fork.get("approval_policy") != "all_external_contributors":
        plan.add(Step(f"fork pull requests: approval policy is {fork.get('approval_policy')!r}, "
                      "want 'all_external_contributors'", "PUT",
                      "actions/permissions/fork-pr-contributor-approval",
                      {"approval_policy": "all_external_contributors"}))


def reviewer_id(login: str) -> int:
    try:
        user = json.loads(gh(["api", f"users/{login}"]))
    except GhError as e:
        raise GhError(f"could not look up reviewer {login!r}: {e}", e.status) from e
    if user.get("type", "User") != "User" or "id" not in user:
        raise GhError(f"{login!r} is not a user account")
    return user["id"]


def make_plan(repo: str, login: str) -> Plan:
    rid = reviewer_id(login)
    plan = Plan()
    immutable_releases(repo, plan)
    tag_ruleset(repo, plan)
    environment(repo, rid, login, plan)
    actions(repo, plan)
    return plan


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true",
                    help="read-only: list every drift and exit 1 if there is any")
    ap.add_argument("--repo", help="OWNER/NAME (default: what `gh repo view` reports)")
    ap.add_argument("--reviewer", default=DEFAULT_REVIEWER,
                    help=f"required reviewer for the {ENVIRONMENT!r} environment "
                         f"(default {DEFAULT_REVIEWER})")
    args = ap.parse_args(argv)

    try:
        repo = args.repo or gh(["repo", "view", "--json", "nameWithOwner", "--jq",
                                ".nameWithOwner"]).strip()
        if not repo:
            raise GhError("could not work out the repository; pass --repo OWNER/NAME")
        plan = make_plan(repo, args.reviewer)
        if args.check:
            if not plan.drifts:
                print(f"{repo}: every setting matches.")
                return 0
            print(f"{repo}: {len(plan.drifts)} setting(s) drifted:")
            for d in plan.drifts:
                print(f"  - {d}")
            return 1
        if not plan.steps:
            print(f"{repo}: nothing to change.")
            return 0
        for d in plan.drifts:
            print(f"  - {d}")
        for s in plan.steps:
            print(f"{s.method} repos/{repo}/{s.path}")
            gh(["api", "-X", s.method, f"repos/{repo}/{s.path}"], s.body)
        left = make_plan(repo, args.reviewer)
        if left.drifts:
            print("still drifted after applying:", file=sys.stderr)
            for d in left.drifts:
                print(f"  - {d}", file=sys.stderr)
            return 1
        print(f"{repo}: applied {len(plan.steps)} change(s); every setting now matches.")
        return 0
    except GhError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
