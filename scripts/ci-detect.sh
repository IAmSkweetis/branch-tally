#!/bin/sh
# CI: decide which Rust jobs have anything to check yet, and say so when they don't.
#
# Before the Cargo workspace lands (#3), the Rust jobs still run and report green, so the required
# status checks exist from day one and switch on by themselves when Cargo.toml appears. A skipped
# check is announced with a ::notice:: so it is visible on the run, never silent.
#
# A pull request that changes nothing Rust-relevant (CODE_CHANGED=false, from the `changes` job in
# ci.yml) skips the same way, with its own notice. Only the exact string "false" skips: empty
# (a push, or the `changes` job failed) and anything else run the checks.
#
# Writes to $GITHUB_OUTPUT (stdout when unset):
#   rust=true|false   Cargo.toml exists
#   deny=true|false   Cargo.toml and deny.toml exist
#   msrv=<version>    the workspace's rust-version, empty when undeclared
set -eu

out="${GITHUB_OUTPUT:-/dev/stdout}"

if [ "${CODE_CHANGED:-}" = "false" ]; then
  {
    echo "rust=false"
    echo "deny=false"
    echo "msrv="
  } >>"$out"
  echo "::notice title=Rust checks skipped::no Rust-relevant file changed in this pull request (docs, Markdown, .claude/ or another workflow only)"
elif [ -f Cargo.toml ]; then
  echo "rust=true" >>"$out"
  # `rust-version` sits in the root manifest ([workspace.package] or [package]).
  msrv=$(sed -n 's/^rust-version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
  echo "msrv=$msrv" >>"$out"
  if [ -f deny.toml ]; then
    echo "deny=true" >>"$out"
  else
    echo "deny=false" >>"$out"
    echo "::notice title=cargo deny skipped::no deny.toml, so cargo deny is skipped until the Workspace Config Steward adds it"
  fi
else
  {
    echo "rust=false"
    echo "deny=false"
    echo "msrv="
  } >>"$out"
  echo "::notice title=Rust checks skipped::no Cargo.toml, Rust checks skipped until the workspace lands (#3)"
fi
