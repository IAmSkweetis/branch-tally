//! End-to-end: `--version` reports the build-time `BRANCH_TALLY_VERSION`, or `0.0.0-dev`.
//!
//! The test binary is built without the variable, so the unset case is what runs here. The set
//! case is covered by the unit test of the selection function in `src/cli.rs`.

use std::process::Command;

#[test]
fn version_flag_prints_dev_version_when_build_env_is_unset() {
    let out = Command::new(env!("CARGO_BIN_EXE_branch-tally"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "branch-tally 0.0.0-dev\n"
    );
}
