//! Source locations, diagnostic data, and deterministic report rendering.

mod outputs;
mod render;
mod source_map;

pub use outputs::{render_github, render_sarif};
pub use render::{render_concise, render_json, render_text, sorted_diagnostics};
pub use source_map::SourceMap;

use serde::{Deserialize, Serialize};

/// A half-open byte range in the original UTF-8 source.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// One-based row and Unicode scalar column in the original source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Location {
    pub row: usize,
    pub column: usize,
}

impl Default for Location {
    fn default() -> Self {
        Self { row: 1, column: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Applicability {
    Safe,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Edit {
    pub byte_range: Span,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Fix {
    pub message: String,
    pub applicability: Applicability,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RelatedLocation {
    pub filename: String,
    pub location: Location,
    pub end_location: Location,
    pub byte_range: Span,
    pub message: String,
}

impl RelatedLocation {
    pub fn new(
        filename: impl Into<String>,
        source: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        let source_map = SourceMap::new(source);
        let byte_range = source_map.clamp_span(span);
        let (location, end_location) = source_map.span_locations(byte_range);
        Self {
            filename: filename.into(),
            location,
            end_location,
            byte_range,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub filename: String,
    pub location: Location,
    pub end_location: Location,
    pub byte_range: Span,
    pub fix: Option<Fix>,
    pub url: Option<String>,
    pub related: Vec<RelatedLocation>,
    pub suggestion: String,
    /// Internal identity used to build suppression fixes before rendering.
    #[serde(skip)]
    pub(crate) unused_suppression_code: Option<String>,
}

impl Diagnostic {
    pub fn new(
        filename: impl Into<String>,
        source: &str,
        code: impl Into<String>,
        span: Span,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        let source_map = SourceMap::new(source);
        let byte_range = source_map.clamp_span(span);
        let (location, end_location) = source_map.span_locations(byte_range);
        let code = code.into();
        let url = crate::rules::rule(&code).map(|rule| rule.url());
        Self {
            code,
            message: message.into(),
            filename: filename.into(),
            location,
            end_location,
            byte_range,
            fix: None,
            url,
            related: Vec::new(),
            suggestion: suggestion.into(),
            unused_suppression_code: None,
        }
    }
}
