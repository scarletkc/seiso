use std::{collections::BTreeMap, fmt::Write};

use serde_json::{Value, json};

use crate::diagnostics::{Applicability, Diagnostic, Location, Span, sorted_diagnostics};

/// Render SARIF 2.1.0 with Unicode scalar columns and UTF-8 byte offsets.
pub fn render_sarif(diagnostics: &[Diagnostic]) -> Result<String, serde_json::Error> {
    let diagnostics = sorted_diagnostics(diagnostics);
    let mut descriptions = BTreeMap::new();
    for diagnostic in &diagnostics {
        descriptions
            .entry(diagnostic.code.clone())
            .or_insert_with(|| diagnostic.url.clone());
    }
    let rules: Vec<_> = descriptions
        .iter()
        .map(|(code, url)| {
            let mut rule = json!({"id": code});
            if let Some(url) = url {
                rule["helpUri"] = json!(url);
            }
            rule
        })
        .collect();
    let rule_indices: BTreeMap<_, _> = descriptions
        .keys()
        .enumerate()
        .map(|(index, code)| (code.as_str(), index))
        .collect();
    let results: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut result = json!({
                "ruleId": diagnostic.code,
                "ruleIndex": rule_indices[diagnostic.code.as_str()],
                "level": "error",
                "message": {"text": message(diagnostic)},
                "locations": [{"physicalLocation": physical_location(
                    &diagnostic.filename, diagnostic.location,
                    diagnostic.end_location, diagnostic.byte_range
                )}],
            });
            if !diagnostic.related.is_empty() {
                result["relatedLocations"] = json!(diagnostic.related.iter().enumerate().map(|(index, related)| json!({
                    "id": index + 1,
                    "physicalLocation": physical_location(&related.filename, related.location, related.end_location, related.byte_range),
                    "message": {"text": related.message},
                })).collect::<Vec<_>>());
            }
            if let Some(fix) = diagnostic.fix.as_ref().filter(|fix| {
                fix.applicability == Applicability::Safe && !fix.edits.is_empty()
            }) {
                let mut edits = fix.edits.clone();
                edits.sort();
                edits.dedup();
                result["fixes"] = json!([{
                    "description": {"text": fix.message},
                    "artifactChanges": [{
                        "artifactLocation": {"uri": artifact_uri(&diagnostic.filename)},
                        "replacements": edits.iter().map(|edit| json!({
                            "deletedRegion": {"byteOffset": edit.byte_range.start, "byteLength": edit.byte_range.len()},
                            "insertedContent": {"binary": base64(edit.content.as_bytes())},
                        })).collect::<Vec<_>>(),
                    }],
                }]);
            }
            result
        })
        .collect();
    let mut output = serde_json::to_string_pretty(&json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "seiso",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/scarletkc/seiso",
                "rules": rules,
                "properties": {"conventionVersion": crate::SPECIFICATION_VERSION},
            }},
            "columnKind": "unicodeCodePoints",
            "defaultEncoding": "utf-8",
            "results": results,
        }],
    }))?;
    output.push('\n');
    Ok(output)
}

fn physical_location(filename: &str, start: Location, end: Location, span: Span) -> Value {
    json!({
        "artifactLocation": {"uri": artifact_uri(filename)},
        "region": {
            "startLine": start.row,
            "startColumn": start.column,
            "endLine": end.row,
            "endColumn": end.column,
            "byteOffset": span.start,
            "byteLength": span.len(),
        },
    })
}

fn artifact_uri(filename: &str) -> String {
    let path = filename.replace('\\', "/");
    let drive = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(2) == Some(&b'/');
    let prefix = if drive {
        "file:///"
    } else if path.starts_with('/') {
        "file://"
    } else {
        ""
    };
    let path = path.strip_prefix("//").unwrap_or(&path);
    let mut uri = prefix.to_owned();
    for (index, byte) in path.bytes().enumerate() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
            || (drive && index == 1 && byte == b':')
        {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

/// Render escaped GitHub Actions error annotation commands, one per diagnostic.
pub fn render_github(diagnostics: &[Diagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in sorted_diagnostics(diagnostics) {
        let _ = write!(
            output,
            "::error file={},line={},endLine={},title={}",
            escape_property(&diagnostic.filename),
            diagnostic.location.row,
            diagnostic.end_location.row,
            escape_property(&diagnostic.code),
        );
        // GitHub accepts columns only for annotations on a single line.
        if diagnostic.location.row == diagnostic.end_location.row {
            let _ = write!(
                output,
                ",col={},endColumn={}",
                diagnostic.location.column,
                diagnostic
                    .end_location
                    .column
                    .saturating_sub(1)
                    .max(diagnostic.location.column)
            );
        }
        let mut text = message(&diagnostic);
        for related in &diagnostic.related {
            let _ = write!(
                text,
                "\nRelated: {}:{}:{}: {}",
                related.filename, related.location.row, related.location.column, related.message
            );
        }
        let _ = writeln!(output, "::{}", escape_data(&text));
    }
    output
}

fn message(diagnostic: &Diagnostic) -> String {
    let mut message = diagnostic.message.clone();
    if !diagnostic.suggestion.is_empty() {
        let _ = write!(message, "\nSuggestion: {}", diagnostic.suggestion);
    }
    if let Some(url) = &diagnostic.url {
        let _ = write!(message, "\nHelp: {url}");
    }
    message
}

fn escape_data(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(value: &str) -> String {
    escape_data(value).replace(':', "%3A").replace(',', "%2C")
}

// SARIF byte-addressed replacements require base64 binary content, even when
// those bytes encode text. Edit offsets refer to the original UTF-8 source.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        result.push(char::from(ALPHABET[usize::from(first >> 2)]));
        result.push(char::from(
            ALPHABET[usize::from((first & 3) << 4 | second >> 4)],
        ));
        result.push(if chunk.len() > 1 {
            char::from(ALPHABET[usize::from((second & 15) << 2 | third >> 6)])
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            char::from(ALPHABET[usize::from(third & 63)])
        } else {
            '='
        });
    }
    result
}
