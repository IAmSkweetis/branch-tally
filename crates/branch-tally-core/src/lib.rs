//! Core library for branch-tally: sources, attribution, ledger, pricing and rendering.
//!
//! Stages run in one direction: source, attribution, ledger, pricing, render. Every stage shares
//! the types in [`model`]. Nothing here looks up a home directory or an environment variable;
//! callers pass the directories in.

pub mod attribution;
pub mod git;
pub mod model;
pub mod pr;
pub mod pricing;
pub mod render;
pub mod report;
pub mod sources;
pub mod summary;
