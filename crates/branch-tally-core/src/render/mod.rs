//! Renders a summary as markdown or JSON.
//!
//! Ports `render`, `wrap`, `splice` and `as_json` from `reference/token_report.py`, byte for byte.
//! Everything is read from the precomputed [`Summary`](crate::model::Summary) and
//! [`Costs`](crate::model::Costs): this stage sorts and formats, it never sums or prices.

mod json;
mod markdown;
mod project;

pub use json::json;
pub use markdown::{END, HEADING, START, markdown, splice, wrap};
pub use project::{project_json, project_markdown, project_svg};
