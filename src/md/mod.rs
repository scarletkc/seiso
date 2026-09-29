//! Markdown content facts with byte ranges into the original UTF-8 source.
//!
//! File paths, effective kinds, resolved filesystem links, and rule results are
//! deliberately absent: those depend on the current workspace, not its content.

mod mapping;
mod parser;
pub(crate) mod prose;

use crate::diagnostics::Span;
use serde::{Deserialize, Serialize};

pub use parser::{parse, parse_with_options};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownFlavor {
    CommonMark,
    #[default]
    Gfm,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParseOptions {
    pub flavor: MarkdownFlavor,
}

#[derive(Debug, thiserror::Error)]
#[error("cannot parse Markdown: {message}")]
pub struct ParseError {
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub source: String,
    pub frontmatter: Option<Frontmatter>,
    /// Section zero covers the document, including content before its first heading.
    pub sections: Vec<Section>,
    /// A flat arena. Parent and child indexes refer to this vector.
    pub blocks: Vec<Block>,
    pub sentences: Vec<Sentence>,
    pub links: Vec<RawLink>,
    pub comments: Vec<HtmlComment>,
    pub identifiers: Vec<Identifier>,
    pub language: Language,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub span: Span,
    /// YAML content without the opening and closing fences.
    pub raw: String,
    /// A declaration only; validity and configured fallback are resolved later.
    pub kind: Option<String>,
    pub lang: Option<String>,
    pub canonical: Option<bool>,
    pub errors: Vec<FrontmatterError>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrontmatterError {
    pub message: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: u8,
    pub heading: Option<String>,
    pub heading_span: Option<Span>,
    /// Ends immediately before the next heading at its level or above.
    pub span: Span,
    pub blocks: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Heading,
    Paragraph,
    List,
    ListItem,
    Table,
    TableRow,
    TableCell,
    Code,
    Blockquote,
    Html,
    HtmlComment,
    ThematicBreak,
    Definition,
    FootnoteDefinition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    pub span: Span,
    pub section: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub sentences: Vec<usize>,
    pub ordered: Option<bool>,
    pub checked: Option<bool>,
    pub code_language: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Zh,
    Ja,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::En, Self::Zh, Self::Ja];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
            Self::Ja => "ja",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.as_str() == name)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    pub span: Span,
    pub block: usize,
    pub language: Language,
    pub fragments: Vec<Fragment>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FragmentKind {
    Text,
    InlineCode,
    LinkText,
    LinkDestination,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fragment {
    pub kind: FragmentKind,
    pub text: String,
    /// Original syntax producing this fragment, including inline delimiters.
    pub span: Span,
    /// Decoded text byte ranges mapped to original source. Exact equal-length
    /// segments map linearly; other segments cover their complete syntax range.
    /// Use `source_span` rather than adding a decoded offset to `span.start`.
    pub mapping: Vec<SourceSegment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceSegment {
    pub text: Span,
    pub source: Span,
    /// False means only the containing syntax range is known. Rules may highlight
    /// that range, but must not claim a more precise character offset.
    pub exact: bool,
}

impl SourceSegment {
    fn covered_source(&self, text: &str, range: Span) -> Span {
        if !self.exact || self.text.end - self.text.start != self.source.end - self.source.start {
            return self.source;
        }
        let mut start = self.text.start.max(range.start);
        let mut end = self.text.end.min(range.end);
        // A byte range inside a UTF-8 character still refers to that whole character.
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        while !text.is_char_boundary(end) {
            end += 1;
        }
        Span::new(
            self.source.start + start - self.text.start,
            self.source.start + end - self.text.start,
        )
    }
}

impl Fragment {
    /// Smallest mapped source range covering the requested decoded text bytes.
    pub fn source_span(&self, range: Span) -> Option<Span> {
        if range.start >= range.end || range.end > self.text.len() {
            return None;
        }
        let mut matches = self
            .mapping
            .iter()
            .filter(|segment| segment.text.start < range.end && range.start < segment.text.end)
            .map(|segment| segment.covered_source(&self.text, range));
        let first = matches.next()?;
        Some(matches.fold(first, |span, segment| Span {
            start: span.start.min(segment.start),
            end: span.end.max(segment.end),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawLink {
    pub span: Span,
    pub destination: String,
    /// Reference links point to the definition's destination span.
    pub destination_span: Span,
    pub title: Option<String>,
    pub reference: Option<String>,
    pub image: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HtmlComment {
    pub span: Span,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Identifier {
    pub text: String,
    pub span: Span,
    pub block: usize,
    pub section: usize,
}
