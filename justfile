# branch-tally command entrypoint. `just` lists the recipes.
#
# Recipes run under PowerShell 7 on Windows and `sh` elsewhere, so every recipe line is a plain
# tool call that reads the same in both. Shell-specific work lives in the private `_require*`
# helpers, written once under [windows] and once under [unix].
#
# CI and the git hooks call these same recipes, so a local `just ci` checks what CI checks.

set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

# Pinned tool versions. CI reads them back with `just --evaluate <name>`, so local and CI agree.
# Bump one here and nowhere else (gitleaks is installed from a release archive whose sha256 is
# pinned in .github/workflows/ci.yml, so bump that checksum with it).
rumdl_version := "0.3.2"
zizmor_version := "1.31.1"
gitleaks_version := "8.30.1"

# Rust checks run only once the workspace exists (#3). Until then they say so and exit 0.
has_cargo := path_exists(justfile_directory() / "Cargo.toml")
skip_rust := "echo 'no Cargo.toml: Rust checks skipped until the workspace lands (#3)'"

# List the recipes.
default:
    @{{ just_executable() }} --list

# Every gate CI runs (everything except fmt and audit-workflows).
ci: lint lint-md test test-scripts secret-scan

# Prove the commit-message and branch-name checks reject what they must and accept what they should.
[unix]
test-scripts: (_require "sh" "install a POSIX sh")
    sh scripts/test-checks.sh

# Git for Windows puts git.exe on PATH but usually not sh.exe, so fall back to the sh it ships
# (<git>/bin/sh.exe, three levels above `git --exec-path`). Git runs the hooks with that same sh.
[windows]
test-scripts: (_require "git" "install Git for Windows: https://gitforwindows.org")
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh scripts/test-checks.sh; exit $LASTEXITCODE

# nextest over the workspace, then doctests (nextest does not run them). Zero tests is a failure.
test filter="": (_require-rust "cargo-nextest" "install it: cargo install cargo-nextest --locked")
    {{ if has_cargo == "true" { "cargo nextest run --workspace --no-tests=fail " + filter } else { skip_rust } }}
    {{ if has_cargo == "true" { "cargo test --workspace --doc " + filter } else { skip_rust } }}

# Format check and Clippy with warnings denied.
lint: (_require-rust "cargo" "install Rust: https://rustup.rs")
    {{ if has_cargo == "true" { "cargo fmt --all --check" } else { skip_rust } }}
    {{ if has_cargo == "true" { "cargo clippy --workspace --all-targets -- -D warnings" } else { skip_rust } }}

# Format the Rust code.
fmt: (_require-rust "cargo" "install Rust: https://rustup.rs")
    {{ if has_cargo == "true" { "cargo fmt --all" } else { skip_rust } }}

# Lint Markdown with rumdl (pinned).
lint-md *paths: (_require "uvx" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uvx rumdl@{{ rumdl_version }} check {{ if paths == "" { "." } else { paths } }}

# Format Markdown with rumdl, rewrapping long paragraphs.
fmt-md *paths: (_require "uvx" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uvx rumdl@{{ rumdl_version }} fmt {{ if paths == "" { "." } else { paths } }}

# gitleaks over the full history, redacted.
secret-scan: (_require "gitleaks" "install it: https://github.com/gitleaks/gitleaks#installing")
    gitleaks git --redact --no-banner .

# zizmor over .github/ (pinned). Pass --offline to skip the online audits.
audit-workflows *args: (_require "uvx" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uvx zizmor@{{ zizmor_version }} {{ args }} .github/

# This repository's own PR token report: the Python reference until the CLI can do it (#4).
token-report *args: (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uv run reference/token_report.py {{ args }}

# Fail with an install hint when a tool is missing. Never skips the check.
[private]
[unix]
_require tool hint:
    @command -v {{ tool }} >/dev/null 2>&1 || { echo "error: {{ tool }} not found; {{ hint }}" >&2; exit 1; }

[private]
[windows]
_require tool hint:
    @if (-not (Get-Command {{ tool }} -ErrorAction SilentlyContinue)) { [Console]::Error.WriteLine('error: {{ tool }} not found; {{ hint }}'); exit 1 }

# Same, but only once a Cargo.toml exists.
[private]
[unix]
_require-rust tool hint:
    @[ ! -f Cargo.toml ] || command -v {{ tool }} >/dev/null 2>&1 || { echo "error: {{ tool }} not found; {{ hint }}" >&2; exit 1; }

[private]
[windows]
_require-rust tool hint:
    @if ((Test-Path Cargo.toml) -and -not (Get-Command {{ tool }} -ErrorAction SilentlyContinue)) { [Console]::Error.WriteLine('error: {{ tool }} not found; {{ hint }}'); exit 1 }
