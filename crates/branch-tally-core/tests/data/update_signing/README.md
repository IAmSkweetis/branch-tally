# Test-only signing vectors for `update`

Everything here is generated for tests with throwaway minisign keypairs that have no connection to
the release key. No secret key was committed, and none of this signs a real release.

- `test-public-key.txt`: the throwaway public key (base64).
- `good-<tag>.minisig`: prehashed signature by that key over the test `SHA256SUMS` for `<tag>`,
  trusted comment `branch-tally <tag> SHA256SUMS`.
- `other-file-v1.3.0.minisig`: good signature over a `SHA256SUMS` that does not list the archive.
- `other-key-v1.3.0.minisig`: signed by a second throwaway key.
- `legacy-v1.3.0.minisig`: legacy (non-prehashed `Ed`) signature.
- `wrong-tag-v1.3.0.minisig`: trusted comment names `v1.2.0`.
- `wrong-file-v1.3.0.minisig`: trusted comment names `install.sh`.

The signed `SHA256SUMS` is the SHA-256 of the bytes `fake archive bytes`, two spaces, then
`branch-tally-<tag>-x86_64-unknown-linux-musl.tar.gz` and a newline (see `tests/update.rs`).
