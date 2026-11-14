"""A stand-in for the `gh` CLI that behaves like the slice of the GitHub API repo_settings.py uses.

It keeps the repository's settings in a JSON file (FAKE_GH_STATE) and appends every call to a log
(FAKE_GH_LOG), one JSON object per line. GETs read the state; PUT, POST and DELETE change it, so a
second run of the script sees what the first run did. Nothing here touches the network.
"""

from __future__ import annotations

import json
import os
import sys


def main(argv: list[str]) -> int:
    state_path = os.environ["FAKE_GH_STATE"]
    with open(state_path, encoding="utf-8") as fh:
        st = json.load(fh)

    if argv[:2] == ["repo", "view"]:
        print(st["repo"])
        return 0
    if argv[:1] != ["api"]:
        print(f"fake gh: unsupported command {argv}", file=sys.stderr)
        return 2

    method, path, body = "GET", None, None
    rest = argv[1:]
    i = 0
    while i < len(rest):
        a = rest[i]
        if a == "-X":
            method = rest[i + 1]
            i += 2
        elif a == "--input":
            body = json.load(sys.stdin) if rest[i + 1] == "-" else None
            i += 2
        else:
            path = a
            i += 1
    path = path.split("?")[0]
    with open(os.environ["FAKE_GH_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"method": method, "path": path, "body": body}) + "\n")

    def out(obj, code=0):
        if obj is not None:
            print(json.dumps(obj))
        return code

    def fail(status, msg):
        print(f"gh: {msg} (HTTP {status})", file=sys.stderr)
        return 1

    def save():
        with open(state_path, "w", encoding="utf-8") as fh:
            json.dump(st, fh)

    if path.startswith("users/"):
        login = path.split("/", 1)[1]
        if login in st["users"]:
            return out({"login": login, "id": st["users"][login], "type": "User"})
        return fail(404, "Not Found")

    prefix = f"repos/{st['repo']}/"
    if not path.startswith(prefix):
        return fail(404, "Not Found")
    p = path[len(prefix):]

    if p == "immutable-releases":
        if method == "PUT":
            st["immutable"] = True
            save()
            return 0
        return out({"enabled": st["immutable"], "enforced_by_owner": False})

    if p == "rulesets" and method == "GET":
        return out([{"id": r["id"], "name": r["name"], "target": r["target"],
                     "enforcement": r["enforcement"]} for r in st["rulesets"]])
    if p == "rulesets" and method == "POST":
        new = dict(body)
        new["id"] = max([r["id"] for r in st["rulesets"]] + [100]) + 1
        st["rulesets"].append(new)
        save()
        return out(new)
    if p.startswith("rulesets/"):
        rid = int(p.split("/")[1])
        idx = next((n for n, r in enumerate(st["rulesets"]) if r["id"] == rid), None)
        if idx is None:
            return fail(404, "Not Found")
        if method == "PUT":
            new = dict(body)
            new["id"] = rid
            st["rulesets"][idx] = new
            save()
            return out(new)
        if method == "DELETE":
            del st["rulesets"][idx]
            save()
            return 0
        return out(st["rulesets"][idx])

    if p == "environments/release":
        env = st.get("environment")
        if method == "PUT":
            policy = body.get("deployment_branch_policy")
            st["environment"] = {
                "reviewers": [{"type": r["type"], "id": r["id"]} for r in body.get("reviewers", [])],
                "prevent_self_review": body.get("prevent_self_review", False),
                "can_admins_bypass": body.get("can_admins_bypass", True),
                "deployment_branch_policy": policy,
                "policies": (env or {}).get("policies", []),
            }
            save()
            return out({"name": "release"})
        if env is None:
            return fail(404, "Not Found")
        rules = []
        if env["reviewers"]:
            rules.append({"type": "required_reviewers",
                          "prevent_self_review": env["prevent_self_review"],
                          "reviewers": [{"type": r["type"], "reviewer": {"id": r["id"], "login": "x"}}
                                        for r in env["reviewers"]]})
        if env["deployment_branch_policy"]:
            rules.append({"type": "branch_policy"})
        return out({"name": "release", "can_admins_bypass": env["can_admins_bypass"],
                    "protection_rules": rules,
                    "deployment_branch_policy": env["deployment_branch_policy"]})

    if p == "environments/release/deployment-branch-policies" or \
            p.startswith("environments/release/deployment-branch-policies/"):
        env = st.get("environment")
        if env is None:
            return fail(404, "Not Found")
        pols = env["policies"]
        if method == "GET":
            return out({"total_count": len(pols), "branch_policies": pols})
        if method == "POST":
            pols.append({"id": max([x["id"] for x in pols] + [500]) + 1, "name": body["name"],
                         "type": body.get("type", "branch")})
            save()
            return out(pols[-1])
        if method == "DELETE":
            pid = int(p.rsplit("/", 1)[1])
            env["policies"] = [x for x in pols if x["id"] != pid]
            save()
            return 0

    a = st["actions"]
    if p == "actions/permissions":
        if method == "PUT":
            a["permissions"] = {"enabled": body["enabled"],
                                "allowed_actions": body.get("allowed_actions", "all"),
                                "sha_pinning_required": body.get("sha_pinning_required", False)}
            save()
            return 0
        return out(a["permissions"])
    if p == "actions/permissions/selected-actions":
        if a["permissions"]["allowed_actions"] != "selected":
            return fail(409, "All actions and workflows are allowed on this repository")
        if method == "PUT":
            a["selected"] = body
            save()
            return 0
        return out(a["selected"])
    if p == "actions/permissions/fork-pr-contributor-approval":
        if method == "PUT":
            a["fork_approval"] = body["approval_policy"]
            save()
            return 0
        return out({"approval_policy": a["fork_approval"]})

    return fail(404, "Not Found")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
