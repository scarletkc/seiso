//! Markdown analysis, configuration, diagnostics, and documentation rules.
//!
//! This library serves the `seiso` command-line tool, and its API can change
//! in any release.

pub mod analysis;
pub mod cache;
pub mod config;
pub mod diagnostics;
pub mod index;
pub mod md;
pub mod paths;
pub mod rules;
pub mod sections;
pub mod workspace;

/// The released version of the Seiso Convention Specification that this
/// build's rules implement. A specification release updates it, and a test
/// keeps it equal to the newest version in `spec/CHANGELOG.md`.
pub const SPECIFICATION_VERSION: &str = "0.1.1";
