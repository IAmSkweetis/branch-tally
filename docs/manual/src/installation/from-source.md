# Build from source

branch-tally is not published to crates.io yet, so `cargo install branch-tally` does not work.
Build it from a clone of the repository instead.

## Requirements

- [rustup](https://rustup.rs/). The repository pins its Rust toolchain (1.99.1) in
  `rust-toolchain.toml`, and rustup installs that version automatically the first time you build
  inside the clone.
- A C compiler. One dependency (`ring`, used for TLS by `prices update`) compiles C code.
- `git`.

## Build and install

```sh
git clone https://github.com/IAmSkweetis/branch-tally.git
cd branch-tally
cargo install --path crates/branch-tally --locked
```

`--locked` builds with the exact dependency versions in the repository's `Cargo.lock`. cargo puts
the binary in `~/.cargo/bin` (`%USERPROFILE%\.cargo\bin` on Windows), which rustup normally adds to
your `PATH`. Pass `--root <dir>` to install somewhere else; the binary goes in `<dir>/bin`.

To build without installing:

```sh
cargo build --release --locked -p branch-tally
```

The binary is then `target/release/branch-tally` (`.exe` on Windows).

## The version of a source build

A source build reports its version as `0.0.0-dev`:

```text
$ branch-tally --version
branch-tally 0.0.0-dev
```

Release builds get their version from the release tag at build time. To build a specific
release, check out its tag first (`git checkout v0.6.1`); the version string still reads
`0.0.0-dev`.

`main` may hold changes that are not in a release yet. This manual documents `main`.
