#!/bin/sh
# CI: install the Rust toolchain for a job.
#
#   ci-install-rust.sh            the repository's pin (rust-toolchain.toml), else stable
#   ci-install-rust.sh <version>  an exact toolchain, used for the MSRV job; exported through
#                                 $GITHUB_ENV so RUSTUP_TOOLCHAIN overrides any pin file
set -eu

if [ -n "${1-}" ]; then
  rustup toolchain install "$1" --profile minimal
  echo "RUSTUP_TOOLCHAIN=$1" >>"${GITHUB_ENV:?GITHUB_ENV must be set to pin an MSRV toolchain}"
elif [ -f rust-toolchain.toml ]; then
  rustup show active-toolchain || rustup toolchain install
else
  rustup toolchain install stable --profile minimal --component rustfmt --component clippy
  rustup default stable
fi
rustc --version
cargo --version
