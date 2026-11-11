#!/usr/bin/env sh
# Checks the shipped plugin and marketplace manifests. Run from the repository root; `just
# check-plugin` builds the binary and runs this.
#   plugin/tests/check.sh [path-to-branch-tally-binary]
# 1. Every manifest is valid JSON, and every hook command ends in `; exit 0`, so no failure can
#    reach Claude Code as a blocking exit 2 (clap usage errors exit 2). Needs only `uv`.
# 2. Every `branch-tally <subcommand>` named in a hook command is a real subcommand of the binary.
#    Without a binary this is an explicit SKIP; with a path that doesn't exist it is a failure.
# 3. `claude plugin validate` passes on the marketplace and on each plugin. Without the `claude`
#    CLI this is an explicit SKIP (CI has none), never a silent pass.
set -u
fail=0
bin="${1:-}"
err() { echo "FAIL: $*" >&2; fail=1; }

if [ -n "$bin" ] && [ ! -f "$bin" ] && [ -f "$bin.exe" ]; then bin="$bin.exe"; fi
if [ -n "$bin" ] && [ ! -f "$bin" ]; then err "binary not found: $bin"; bin=""; fi

command -v uv >/dev/null 2>&1 || { echo "error: uv not found; install uv: https://docs.astral.sh/uv/getting-started/installation/" >&2; exit 1; }

[ -d plugin/capture-hooks ] || err "plugin/capture-hooks missing"
[ -d plugin/record-hooks ] || err "plugin/record-hooks missing"

hooks_files=""
for dir in plugin/*-hooks; do
  [ -d "$dir" ] || continue
  f="$dir/hooks/hooks.json"
  [ -f "$f" ] || { err "$dir has no hooks/hooks.json"; continue; }
  hooks_files="$hooks_files $f"
done
manifests=".claude-plugin/marketplace.json $(ls plugin/*-hooks/.claude-plugin/plugin.json 2>/dev/null | tr '\n' ' ')"

# Parse every manifest; print one `SUB <name>` line per subcommand a hook names, and report bad
# JSON or a hook command that can exit non-zero on stderr.
# shellcheck disable=SC2086
subs=$(uv run python -I - $manifests -- $hooks_files <<'PY'
import json, re, sys

args = sys.argv[1:]
split = args.index("--")
manifests, hooks_files = args[:split], args[split + 1:]
bad = False
for path in manifests + hooks_files:
    try:
        with open(path, encoding="utf-8") as fh:
            doc = json.load(fh)
    except (OSError, ValueError) as exc:
        print(f"FAIL: {path} is not valid JSON: {exc}", file=sys.stderr)
        bad = True
        continue
    if path in hooks_files:
        count = 0
        for groups in doc.get("hooks", {}).values():
            for group in groups:
                for hook in group.get("hooks", []):
                    cmd = hook.get("command", "")
                    count += 1
                    if not cmd.rstrip().endswith("; exit 0"):
                        print(f"FAIL: {path}: hook command can exit non-zero: {cmd}", file=sys.stderr)
                        bad = True
                    for sub in re.findall(r"branch-tally\s+([a-z][a-z-]*)", cmd):
                        print(f"SUB {sub}")
        if count == 0:
            print(f"FAIL: {path} defines no hook commands", file=sys.stderr)
            bad = True
sys.exit(1 if bad else 0)
PY
) || fail=1

if [ -n "$bin" ]; then
  for sub in $(echo "$subs" | awk '$1=="SUB"{print $2}' | sort -u); do
    "$bin" "$sub" --help >/dev/null 2>&1 || err "hooks name unknown subcommand: $sub"
  done
else
  echo "SKIP: no binary given, subcommand names not checked"
fi

if command -v claude >/dev/null 2>&1; then
  claude plugin validate . || err "marketplace did not validate"
  for dir in plugin/*-hooks; do
    [ -d "$dir" ] || continue
    claude plugin validate "$dir" || err "$dir did not validate"
  done
else
  echo "SKIP: claude CLI not found: manifest validation skipped"
fi

[ "$fail" -eq 0 ] && echo "plugin check: ok"
exit "$fail"
