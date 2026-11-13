# branch-tally command entrypoint. `just` lists the recipes.
#
# Recipes run under PowerShell 7 on Windows and `sh` elsewhere, so every recipe line is a plain
# tool call that reads the same in both. Shell-specific work lives in private `_helper` recipes,
# written once under [windows] and once under [unix]. Needs just 1.56+ (`[windows] set shell`).
#
# CI and the git hooks call these same recipes, so a local `just ci` checks what CI checks.

[windows]
set shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

# Pinned tool versions. CI reads them back with `just --evaluate <name>`, so local and CI agree.
# mise.toml mirrors them for local setup; `just check-pins` fails when the two drift.
# Bump one here and nowhere else (gitleaks is installed from a release archive whose sha256 is
# pinned in .github/workflows/ci.yml, so bump that checksum with it; mdbook and mdbook-mermaid are
# the same, with their checksums in .github/workflows/docs.yml and pages.yml, and the two must stay
# compatible: mdbook-mermaid 0.17 is built against the mdBook 0.5 preprocessor API).
rumdl_version := "0.3.2"
zizmor_version := "1.31.1"
gitleaks_version := "8.30.1"
cliff_version := "2.13.1"
mdbook_version := "0.5.4"
mdbook_mermaid_version := "0.17.1"

# Rust checks run only once the workspace exists (#3). Until then they say so and exit 0.
has_cargo := path_exists(justfile_directory() / "Cargo.toml")
skip_rust := "echo 'no Cargo.toml: Rust checks skipped until the workspace lands (#3)'"

# List the recipes.
default:
    @{{ just_executable() }} --list

# Every gate CI runs (everything except fmt and audit-workflows).
ci: lint lint-md test test-scripts test-py check-pins check-plugin lint-install test-install docs secret-scan

# Prove the commit-message and branch-name checks reject what they must and accept what they should.
[unix]
test-scripts: (_require "sh" "install a POSIX sh")
    sh scripts/test-checks.sh

# Git for Windows puts git.exe on PATH but usually not sh.exe, so fall back to the sh it ships
# (<git>/bin/sh.exe, three levels above `git --exec-path`). Git runs the hooks with that same sh.
[windows]
test-scripts: (_require "git" "install Git for Windows: https://gitforwindows.org")
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh scripts/test-checks.sh; exit $LASTEXITCODE

# ShellCheck on install.sh and its test, PSScriptAnalyzer on the .ps1 files. A missing tool is a printed skip locally and an error when CI is set.
[unix]
lint-install: (_require "sh" "install a POSIX sh")
    sh scripts/lint-install.sh

# Same sh lookup as test-scripts.
[windows]
lint-install: (_require "git" "install Git for Windows: https://gitforwindows.org")
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh scripts/lint-install.sh; exit $LASTEXITCODE

# Run the installers' offline tests under sh and pwsh. A missing pwsh is a printed skip locally and an error when CI is set.
[unix]
test-install: (_require "sh" "install a POSIX sh")
    sh scripts/test-install.sh

# Same sh lookup as test-scripts.
[windows]
test-install: (_require "git" "install Git for Windows: https://gitforwindows.org")
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh scripts/test-install.sh; exit $LASTEXITCODE

# Run install.ps1's downloader against real GitHub (needs the network, so not part of ci): pwsh, or powershell for Windows PowerShell 5.1.
test-install-live shell="pwsh": (_require shell "install PowerShell 7: https://learn.microsoft.com/powershell/scripting/install")
    {{ shell }} -NoProfile -ExecutionPolicy Bypass -File tests/install/install_live_test.ps1

# Build the user manual (docs/manual) into docs/manual/book. Fails on a SUMMARY.md entry with no page.
docs: (_require "mdbook" ("install it: https://github.com/rust-lang/mdBook/releases/tag/v" + mdbook_version + " or cargo install mdbook --version " + mdbook_version + " --locked")) (_require "mdbook-mermaid" ("install it: https://github.com/badboy/mdbook-mermaid/releases/tag/v" + mdbook_mermaid_version + " or cargo install mdbook-mermaid --version " + mdbook_mermaid_version + " --locked"))
    mdbook build docs/manual

# Serve the user manual locally and rebuild on change (http://localhost:3000).
docs-serve: (_require "mdbook" ("install it: https://github.com/rust-lang/mdBook/releases/tag/v" + mdbook_version + " or cargo install mdbook --version " + mdbook_version + " --locked")) (_require "mdbook-mermaid" ("install it: https://github.com/badboy/mdbook-mermaid/releases/tag/v" + mdbook_mermaid_version + " or cargo install mdbook-mermaid --version " + mdbook_mermaid_version + " --locked"))
    mdbook serve docs/manual

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

# gitleaks over the full history of every local ref, redacted. Warns when the local gitleaks differs
# from CI's pin. CI passes `--log-opts=HEAD` so a PR is judged on main plus its own commits only:
# `fetch-depth: 0` fetches every branch, and one leaky branch would otherwise turn every PR red.
secret-scan *args: (_require "gitleaks" "install it: https://github.com/gitleaks/gitleaks#installing") _gitleaks-version
    gitleaks git --redact --no-banner {{ args }} .

# zizmor over .github/ (pinned). Pass --offline to skip the online audits.
audit-workflows *args: (_require "uvx" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uvx zizmor@{{ zizmor_version }} {{ args }} .github/

# Unit tests for the Python tooling in scripts/ (stdlib unittest via uv; needs git for the e2e test).
test-py: (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/") (_require "git" "install git")
    uv run python -m unittest discover -s scripts/tests

# Fail when mise.toml disagrees with the pins here, in the workflows or in rust-toolchain.toml, or when a vendored manual asset no longer matches its recorded sha256. Needs neither mise nor network.
check-pins: (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uv run scripts/check_pins.py

# Check the shipped plugin and marketplace: manifests parse, every hook command ends in `; exit 0`,
# every hook names a real subcommand of the freshly built binary. `claude plugin validate` also runs
# when the claude CLI is installed, and is a printed skip otherwise (CI has none).
[unix]
check-plugin: (_require "cargo" "install Rust: https://rustup.rs") (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/") (_require "sh" "install a POSIX sh")
    cargo build -p branch-tally
    sh plugin/tests/check.sh target/debug/branch-tally

# Same sh lookup as test-scripts.
[windows]
check-plugin: (_require "cargo" "install Rust: https://rustup.rs") (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/") (_require "git" "install Git for Windows: https://gitforwindows.org")
    cargo build -p branch-tally
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh plugin/tests/check.sh target/debug/branch-tally; exit $LASTEXITCODE

# Scrub a real transcript tree into a committable fixture: just scrub --root <repo root> <src> <dest>
scrub *args: (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/")
    uv run scripts/scrub_transcripts.py {{ args }}

# Regenerate the fixture goldens from reference/token_report.py: just goldens [scenario] [--check]
goldens *args: (_require "uv" "install uv: https://docs.astral.sh/uv/getting-started/installation/") (_require "git" "install git")
    uv run scripts/make_goldens.py {{ args }}

# This repository's own PR token report, from the installed release binary: just token-report --pr
token-report *args: (_require "branch-tally" "install the latest release: https://github.com/IAmSkweetis/branch-tally/releases/latest")
    branch-tally report {{ args }}

# Append this branch's usage to the ledger with the installed release binary: just token-record
token-record *args: (_require "branch-tally" "install the latest release: https://github.com/IAmSkweetis/branch-tally/releases/latest")
    branch-tally record {{ args }}

# Regenerate the README's cost-over-time chart from the ledger, with the installed release binary.
# The SVG's headline is the project total, so this one file keeps the README's chart and total current.
readme-graph: (_require "branch-tally" "install the latest release: https://github.com/IAmSkweetis/branch-tally/releases/latest")
    branch-tally project --export-graph docs/assets/project-cost.svg

# The version the next release would get, from the Conventional Commits since the last v* tag.
# Needs git-cliff at cliff_version and the full history (a shallow clone gets the wrong answer).
next-version: (_require "git-cliff" ("install it: cargo install git-cliff --version " + cliff_version + " --locked"))
    git-cliff --config cliff.toml --bumped-version

# Preview the release notes for the unreleased commits. Writes nothing. Pass a version (v1.0.0) to
# name it, else git-cliff picks the bump.
release-notes version="": (_require "git-cliff" ("install it: cargo install git-cliff --version " + cliff_version + " --locked"))
    git-cliff --config cliff.toml --unreleased {{ if version == "" { "--bump" } else { "--tag " + version } }}

# Package a built binary as a release archive in dist/: just package v0.1.0 <target> [binary].
# See scripts/package.sh. Build first: BRANCH_TALLY_VERSION=0.1.0 cargo build --release --locked.
[unix]
package *args: (_require "sh" "install a POSIX sh")
    sh scripts/package.sh {{ args }}

# Same sh lookup as test-scripts.
[windows]
package *args: (_require "git" "install Git for Windows: https://gitforwindows.org")
    @$sh = (Get-Command sh -ErrorAction SilentlyContinue).Source; if (-not $sh) { $sh = Join-Path (git --exec-path) '../../../bin/sh.exe' }; if (-not (Test-Path $sh)) { [Console]::Error.WriteLine('error: sh not found on PATH or beside git; install Git for Windows: https://gitforwindows.org'); exit 1 }; & $sh scripts/package.sh {{ args }}; exit $LASTEXITCODE

# Fail with an install hint when a tool is missing. Never skips the check.
[private]
[unix]
_require tool hint:
    @command -v {{ tool }} >/dev/null 2>&1 || { echo "error: {{ tool }} not found; {{ hint }}" >&2; exit 1; }

[private]
[windows]
_require tool hint:
    @if (-not (Get-Command {{ tool }} -ErrorAction SilentlyContinue)) { [Console]::Error.WriteLine('error: {{ tool }} not found; {{ hint }}'); exit 1 }

# A warning, not a failure: a package-manager update shouldn't block a commit, and CI runs the pin.
[private]
[unix]
_gitleaks-version:
    @v=$(gitleaks version | sed 's/^v//'); [ "$v" = "{{ gitleaks_version }}" ] || echo "warning: gitleaks $v is installed but CI pins {{ gitleaks_version }}; results may differ" >&2

[private]
[windows]
_gitleaks-version:
    @$v = "$(gitleaks version)".TrimStart('v'); if ($v -ne '{{ gitleaks_version }}') { [Console]::Error.WriteLine("warning: gitleaks $v is installed but CI pins {{ gitleaks_version }}; results may differ") }

# Same, but only once a Cargo.toml exists.
[private]
[unix]
_require-rust tool hint:
    @[ ! -f Cargo.toml ] || command -v {{ tool }} >/dev/null 2>&1 || { echo "error: {{ tool }} not found; {{ hint }}" >&2; exit 1; }

[private]
[windows]
_require-rust tool hint:
    @if ((Test-Path Cargo.toml) -and -not (Get-Command {{ tool }} -ErrorAction SilentlyContinue)) { [Console]::Error.WriteLine('error: {{ tool }} not found; {{ hint }}'); exit 1 }
