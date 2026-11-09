//! Core library for branch-tally: sources, attribution, ledger, pricing and rendering.
//!
//! Stages run in one direction: source, attribution, ledger, pricing, render. Every stage shares
//! the types in [`model`]. Nothing here looks up a home directory or an environment variable;
//! callers pass the directories in.

pub mod model;
pub mod sources;

#[cfg(test)]
mod red_proof {
    #[test]
    fn deliberately_fails() {
        let n = "ab".len();
        assert_eq!(n, 3, "red proof for the test and msrv jobs");
    }
}
