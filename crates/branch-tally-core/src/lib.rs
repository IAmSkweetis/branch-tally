//! Core library for branch-tally: sources, attribution, ledger, pricing and rendering.
//!
//! Stages run in one direction: source, attribution, ledger, pricing, render. Every stage shares
//! the types in [`model`]. Nothing here looks up a home directory or an environment variable;
//! callers pass the directories in.

pub mod attribution;
pub mod config;
pub mod git;
pub mod ledger;
pub mod model;
pub mod paths;
pub mod pr;
pub mod pricing;
pub mod redact;
pub mod render;
pub mod report;
pub mod sources;
pub mod summary;
