//! Comment exemptions and their per-rule execution states.

use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostics::{Diagnostic, Span};
use crate::md::{BlockKind, Document, HtmlComment};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuppressionScope {
    File,
    Block { span: Span },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SuppressionState {
    Active { count: usize },
    Stale,
    NotEvaluated,
    RuleDisabled,
    Incomplete { count: usize },
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuppressionRecord {
    pub codes: Vec<String>,
    pub reason: String,
    pub span: Span,
    pub scope: Option<SuppressionScope>,
    pub states: BTreeMap<String, SuppressionState>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuppressionResult {
    pub diagnostics: Vec<Diagnostic>,
    pub suppressions: Vec<SuppressionRecord>,
}

/// Inspect declarations without running lint rules or inferring their activity.
pub fn inspect(document: &Document, enabled: &BTreeSet<String>) -> Vec<SuppressionRecord> {
    let mut records = declarations(document, enabled, &BTreeSet::new());
    for state in records
        .iter_mut()
        .flat_map(|record| record.states.values_mut())
    {
        if *state == SuppressionState::Stale {
            *state = SuppressionState::NotEvaluated;
        }
    }
    records
}

fn declarations(
    document: &Document,
    enabled: &BTreeSet<String>,
    incomplete: &BTreeSet<String>,
) -> Vec<SuppressionRecord> {
    let mut records: Vec<_> = document
        .comments
        .iter()
        .filter(|comment| comment.content.trim_start().starts_with("seiso:"))
        .map(|comment| declaration(document, comment, enabled, incomplete))
        .collect();
    records.sort_by_key(|record| record.span);
    records
}

/// Apply valid declarations to primary locations, then report unused exemptions.
/// `enabled` contains rules that ran for this file, after all policy filters.
pub fn apply(
    document: &Document,
    filename: &str,
    diagnostics: Vec<Diagnostic>,
    enabled: &BTreeSet<String>,
    incomplete: &BTreeSet<String>,
) -> SuppressionResult {
    let mut suppressions = declarations(document, enabled, incomplete);

    let mut pending = diagnostics;
    if enabled.contains("SUP001") {
        pending.extend(suppressions.iter().filter_map(|record| {
            record.error.as_ref().map(|error| {
                Diagnostic::new(
                    filename,
                    &document.source,
                    "SUP001",
                    record.span,
                    format!("Invalid suppression: {error}"),
                    "Use `<!-- seiso: allow CODE -- reason -->` before a block or inside a paragraph, list item, or table cell; put `allow-file` before the first content block.",
                )
            })
        }));
    }
    let mut retained = Vec::new();
    for diagnostic in pending {
        if !suppress(&diagnostic, filename, None, &mut suppressions) {
            retained.push(diagnostic);
        }
    }

    if enabled.contains("SUP002") && !incomplete.contains("SUP002") {
        // SUP002 declarations consume first-order stale diagnostics before their
        // own state is evaluated. This prevents circular meta-exemptions.
        let stale: Vec<_> = suppressions
            .iter()
            .enumerate()
            .flat_map(|(index, record)| {
                record.states.iter().filter_map(move |(code, state)| {
                    (code != "SUP002" && *state == SuppressionState::Stale)
                        .then_some((index, code.clone()))
                })
            })
            .collect();
        for (index, code) in stale {
            let diagnostic = stale_diagnostic(document, filename, &suppressions[index], &code);
            if !suppress(&diagnostic, filename, Some(index), &mut suppressions) {
                retained.push(diagnostic);
            }
        }
        for record in &suppressions {
            if record.states.get("SUP002") == Some(&SuppressionState::Stale) {
                retained.push(stale_diagnostic(document, filename, record, "SUP002"));
            }
        }
    }
    SuppressionResult {
        diagnostics: retained,
        suppressions,
    }
}

fn declaration(
    document: &Document,
    comment: &HtmlComment,
    enabled: &BTreeSet<String>,
    incomplete: &BTreeSet<String>,
) -> SuppressionRecord {
    let mut record = SuppressionRecord {
        codes: Vec::new(),
        reason: String::new(),
        span: comment.span,
        scope: None,
        states: BTreeMap::new(),
        error: None,
    };
    match parse_declaration(document, comment, &mut record) {
        Ok(scope) => {
            record.scope = Some(scope);
            for code in &record.codes {
                let state = if !enabled.contains(code) {
                    SuppressionState::RuleDisabled
                } else if incomplete.contains(code) {
                    SuppressionState::Incomplete { count: 0 }
                } else {
                    SuppressionState::Stale
                };
                record.states.insert(code.clone(), state);
            }
        }
        Err(error) => {
            record.error = Some(error);
            record.states = record
                .codes
                .iter()
                .map(|code| (code.clone(), SuppressionState::Invalid))
                .collect();
        }
    }
    record
}

fn parse_declaration(
    document: &Document,
    comment: &HtmlComment,
    record: &mut SuppressionRecord,
) -> Result<SuppressionScope, String> {
    let body = comment
        .content
        .trim()
        .strip_prefix("seiso:")
        .unwrap()
        .trim();
    let Some((command, arguments)) = body.split_once(char::is_whitespace) else {
        return Err("expected a command, full rule codes, and a reason".into());
    };
    if !matches!(command, "allow" | "allow-file") {
        return Err(format!("unknown command `{command}`"));
    }
    let separator = arguments.split_once("--");
    let (codes, reason) = separator.unwrap_or((arguments, ""));
    record.reason = reason.trim().to_owned();
    record.codes = codes
        .split(',')
        .map(|code| code.trim().to_owned())
        .collect();
    if separator.is_none() {
        return Err("the reason must follow `--`".into());
    }
    if comment.content.contains(['\r', '\n']) {
        return Err("the declaration must fit on one line".into());
    }
    if record.reason.is_empty() {
        return Err("a nonempty reason is required".into());
    }
    let mut seen = BTreeSet::new();
    for code in &record.codes {
        if super::rule(code).is_none() {
            return Err(format!("`{code}` is not a known full rule code"));
        }
        if !seen.insert(code) {
            return Err(format!("rule `{code}` is listed more than once"));
        }
    }
    if command == "allow-file" {
        if !standalone(document, comment.span) || !before_content(document, comment.span) {
            return Err("`allow-file` must appear after frontmatter and before all content".into());
        }
        return Ok(SuppressionScope::File);
    }
    if standalone(document, comment.span) {
        return next_block(document, comment.span)
            .map(|span| SuppressionScope::Block { span })
            .ok_or_else(|| "a standalone `allow` must precede a supported content block".into());
    }
    inline_block(document, comment.span)
        .map(|span| SuppressionScope::Block { span })
        .ok_or_else(|| "inline `allow` must be inside a paragraph, list item, or table cell".into())
}

fn standalone(document: &Document, span: Span) -> bool {
    let prefix = &document.source[..span.start];
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let suffix = &document.source[span.end..];
    let line_end = suffix.find('\n').unwrap_or(suffix.len());
    document.source[line_start..span.start].trim().is_empty()
        && suffix[..line_end].trim().is_empty()
}

fn before_content(document: &Document, span: Span) -> bool {
    let mut cursor = document.frontmatter.as_ref().map_or(0, |fm| fm.span.end);
    if span.start < cursor {
        return false;
    }
    for comment in &document.comments {
        if comment.span.start < cursor || comment.span.end > span.start {
            continue;
        }
        if !document.source[cursor..comment.span.start]
            .trim()
            .is_empty()
        {
            return false;
        }
        cursor = comment.span.end;
    }
    document.source[cursor..span.start].trim().is_empty()
}

fn next_block(document: &Document, comment: Span) -> Option<Span> {
    let parent = document
        .blocks
        .iter()
        .filter(|block| block.kind == BlockKind::HtmlComment && contains_span(block.span, comment))
        .min_by_key(|block| block.span.len())
        .and_then(|block| block.parent);
    let next = document
        .blocks
        .iter()
        .filter(|block| {
            block.parent == parent
                && block.span.start >= comment.end
                && block.kind != BlockKind::HtmlComment
        })
        .min_by_key(|block| (block.span.start, usize::MAX - block.span.len()))?;
    matches!(
        next.kind,
        BlockKind::Paragraph
            | BlockKind::List
            | BlockKind::Table
            | BlockKind::Code
            | BlockKind::Heading
    )
    .then_some(next.span)
}

fn inline_block(document: &Document, comment: Span) -> Option<Span> {
    let containing: Vec<_> = document
        .blocks
        .iter()
        .filter(|block| contains_span(block.span, comment))
        .collect();
    containing
        .iter()
        .filter(|block| matches!(block.kind, BlockKind::TableCell | BlockKind::ListItem))
        .min_by_key(|block| block.span.len())
        .or_else(|| {
            containing
                .iter()
                .filter(|block| block.kind == BlockKind::Paragraph)
                .min_by_key(|block| block.span.len())
        })
        .map(|block| block.span)
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn suppress(
    diagnostic: &Diagnostic,
    filename: &str,
    origin: Option<usize>,
    suppressions: &mut [SuppressionRecord],
) -> bool {
    if diagnostic.filename != filename {
        return false;
    }
    let winner = suppressions
        .iter()
        .enumerate()
        .filter(|(index, record)| {
            Some(*index) != origin
                && record.error.is_none()
                && record.states.get(&diagnostic.code).is_some_and(|state| {
                    !matches!(
                        state,
                        SuppressionState::RuleDisabled | SuppressionState::Invalid
                    )
                })
        })
        .filter_map(|(index, record)| match record.scope {
            Some(SuppressionScope::File) => Some((index, (1, usize::MAX, record.span.start))),
            Some(SuppressionScope::Block { span })
                if span.start <= diagnostic.byte_range.start
                    && diagnostic.byte_range.start < span.end =>
            {
                Some((index, (0, span.len(), record.span.start)))
            }
            _ => None,
        })
        .min_by_key(|(_, priority)| *priority)
        .map(|(index, _)| index);
    let Some(index) = winner else {
        return false;
    };
    let state = suppressions[index]
        .states
        .get_mut(&diagnostic.code)
        .unwrap();
    match state {
        SuppressionState::Active { count } | SuppressionState::Incomplete { count } => *count += 1,
        _ => *state = SuppressionState::Active { count: 1 },
    }
    true
}

fn stale_diagnostic(
    document: &Document,
    filename: &str,
    record: &SuppressionRecord,
    code: &str,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        filename,
        &document.source,
        "SUP002",
        record.span,
        format!("Suppression for {code} did not suppress any diagnostic."),
        format!(
            "Remove {code} from the declaration, or remove the comment if it lists no other rules."
        ),
    );
    diagnostic.unused_suppression_code = Some(code.to_owned());
    diagnostic
}
