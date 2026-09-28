use std::{collections::BTreeMap, fmt::Write};

use crate::diagnostics::{Diagnostic, SourceMap};

/// Canonical report order is path, row, column, and code. Remaining fields break
/// ties, so upstream traversal and parallel execution cannot affect the report.
pub fn sorted_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics.to_vec();
    for diagnostic in &mut diagnostics {
        diagnostic.related.sort();
    }
    diagnostics.sort_by(|left, right| {
        (&left.filename, left.location, &left.code)
            .cmp(&(&right.filename, right.location, &right.code))
            .then_with(|| left.cmp(right))
    });
    diagnostics
}

pub fn render_json(diagnostics: &[Diagnostic]) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_string_pretty(&sorted_diagnostics(diagnostics))?;
    json.push('\n');
    Ok(json)
}

pub fn render_concise(diagnostics: &[Diagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in sorted_diagnostics(diagnostics) {
        write_header(&mut output, &diagnostic);
        let _ = write!(output, " Suggestion: {}", one_line(&diagnostic.suggestion));
        for related in diagnostic.related {
            let _ = write!(
                output,
                " Related: {}:{}:{}: {}",
                one_line(&related.filename),
                related.location.row,
                related.location.column,
                one_line(&related.message),
            );
        }
        output.push('\n');
    }
    output
}

pub fn render_text(diagnostics: &[Diagnostic], sources: &BTreeMap<String, String>) -> String {
    let mut output = String::new();
    for diagnostic in sorted_diagnostics(diagnostics) {
        write_header(&mut output, &diagnostic);
        output.push('\n');
        if let Some(source) = sources.get(&diagnostic.filename) {
            let source_map = SourceMap::new(source);
            let (start, end) = source_map.span_locations(diagnostic.byte_range);
            let final_row = if end.row > start.row && end.column == 1 {
                end.row - 1
            } else {
                end.row
            };
            let width = final_row.to_string().len();
            let _ = writeln!(output, " {:width$} |", "");
            for row in start.row..=final_row {
                if let Some(line) = source_map.line(row) {
                    let _ = writeln!(output, " {row:width$} | {line}");
                }
            }
            let _ = writeln!(output, " {:width$} |", "");
        }
        let _ = writeln!(
            output,
            "   = suggestion: {}",
            one_line(&diagnostic.suggestion)
        );
        for related in diagnostic.related {
            let _ = writeln!(
                output,
                "   = related: {}:{}:{}: {}",
                one_line(&related.filename),
                related.location.row,
                related.location.column,
                one_line(&related.message),
            );
        }
        output.push('\n');
    }
    match diagnostics.len() {
        0 => output.push_str("No diagnostics.\n"),
        1 => output.push_str("Found 1 diagnostic.\n"),
        count => {
            let _ = writeln!(output, "Found {count} diagnostics.");
        }
    }
    output
}

fn write_header(output: &mut String, diagnostic: &Diagnostic) {
    let _ = write!(
        output,
        "{}:{}:{}: {} {}",
        one_line(&diagnostic.filename),
        diagnostic.location.row,
        diagnostic.location.column,
        one_line(&diagnostic.code),
        one_line(&diagnostic.message),
    );
}

fn one_line(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}
