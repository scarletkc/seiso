//! Visible prose runs with source mappings and URL masking.

use super::{Fragment, FragmentKind, Sentence};
use crate::diagnostics::Span;
use regex::Regex;
use std::sync::LazyLock;

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)[^\s<>]+").unwrap());
static QUOTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""[^"\n]*"|“[^”\n]*”|‘[^’\n]*’|「[^」\n]*」|『[^』\n]*』"#).unwrap()
});
struct Piece<'a> {
    fragment: &'a Fragment,
    start: usize,
}

#[derive(Default)]
pub(crate) struct Run<'a> {
    pub(crate) text: String,
    pieces: Vec<Piece<'a>>,
}

impl Run<'_> {
    pub(crate) fn span(&self, range: Span) -> Option<Span> {
        self.pieces
            .iter()
            .filter_map(|piece| {
                let start = range.start.max(piece.start).saturating_sub(piece.start);
                let end = range
                    .end
                    .saturating_sub(piece.start)
                    .min(piece.fragment.text.len());
                piece.fragment.source_span(Span::new(start, end))
            })
            .reduce(|left, right| Span::new(left.start.min(right.start), left.end.max(right.end)))
    }
}

pub(crate) fn runs(sentence: &Sentence, include_code: bool) -> Vec<Run<'_>> {
    let mut result = Vec::new();
    let mut run = Run::default();
    for fragment in &sentence.fragments {
        if fragment.kind == FragmentKind::LinkDestination {
            continue;
        }
        if fragment.kind == FragmentKind::InlineCode && !include_code {
            if !run.text.is_empty() {
                result.push(std::mem::take(&mut run));
            }
            continue;
        }
        // Code delimiters separate tokens even when the source omits surrounding spaces.
        if fragment.kind == FragmentKind::InlineCode {
            run.text.push(' ');
        }
        run.pieces.push(Piece {
            fragment,
            start: run.text.len(),
        });
        run.text.push_str(&fragment.text);
        if fragment.kind == FragmentKind::InlineCode {
            run.text.push(' ');
        }
    }
    if !run.text.is_empty() {
        result.push(run);
    }
    mask(&mut result, &URL);
    result
}

/// Quoted examples are not assertions by the surrounding document. Mask bytes
/// rather than deleting text so source mappings retain their offsets.
pub(crate) fn assertions(sentence: &Sentence, include_code: bool) -> Vec<Run<'_>> {
    let mut result = runs(sentence, include_code);
    mask(&mut result, &QUOTATION);
    result
}

fn mask(runs: &mut [Run<'_>], pattern: &Regex) {
    for run in runs {
        let ranges: Vec<_> = pattern
            .find_iter(&run.text)
            .map(|matched| matched.range())
            .collect();
        for range in ranges.into_iter().rev() {
            run.text
                .replace_range(range.clone(), &" ".repeat(range.len()));
        }
    }
}

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

pub(crate) fn occurrences(text: &str, phrase: &str) -> Vec<Span> {
    let lower = text.to_ascii_lowercase();
    let phrase = phrase.trim().to_ascii_lowercase();
    if phrase.is_empty() {
        return Vec::new();
    }
    lower
        .match_indices(&phrase)
        .filter_map(|(start, _)| {
            let end = start + phrase.len();
            let left = phrase.starts_with(|ch: char| ch.is_ascii_alphanumeric())
                && lower[..start].chars().next_back().is_some_and(word_char);
            let right = phrase.ends_with(|ch: char| ch.is_ascii_alphanumeric())
                && lower[end..].chars().next().is_some_and(word_char);
            (!left && !right).then_some(Span::new(start, end))
        })
        .collect()
}

pub(crate) fn marker_if(
    runs: &[Run<'_>],
    phrases: &[&str],
    predicate: impl Fn(&str, &str) -> bool + Copy,
) -> Option<Span> {
    runs.iter()
        .flat_map(|run| {
            phrases.iter().flat_map(move |phrase| {
                occurrences(&run.text, phrase)
                    .into_iter()
                    .filter(move |range| {
                        predicate(&run.text[..range.start], &run.text[range.end..])
                    })
                    .filter_map(|range| run.span(range))
            })
        })
        .min_by_key(|span| (span.start, span.end))
}
