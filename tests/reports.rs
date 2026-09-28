use std::collections::BTreeMap;

use seiso::diagnostics::{
    Applicability, Diagnostic, Edit, Fix, RelatedLocation, SourceMap, Span, render_concise,
    render_json, render_text, sorted_diagnostics,
};

const SOURCE: &str = "# Guide\r\nEnglish \\*text\\* &amp; 中文。\r\n日本語で `設定` を見る。\r\n";

fn diagnostic(needle: &str, code: &str) -> Diagnostic {
    let start = SOURCE.find(needle).unwrap();
    Diagnostic::new(
        "docs/mixed.md",
        SOURCE,
        code,
        Span::new(start, start + needle.len()),
        "Source mapping example.",
        "Inspect the original Markdown range.",
    )
}

fn fixture() -> (Vec<Diagnostic>, BTreeMap<String, String>) {
    let mut japanese = diagnostic("設定", "TST003");
    japanese.related.push(RelatedLocation::new(
        "docs/reference.md",
        "# 設定\n",
        Span::new(2, 8),
        "Related definition.",
    ));
    (
        vec![
            japanese,
            diagnostic("中文", "TST002"),
            diagnostic("text", "TST001"),
        ],
        BTreeMap::from([("docs/mixed.md".to_owned(), SOURCE.to_owned())]),
    )
}

#[test]
fn multilingual_crlf_source_snapshots() {
    let (diagnostics, sources) = fixture();
    insta::assert_snapshot!("multilingual_text", render_text(&diagnostics, &sources));
    insta::assert_snapshot!("multilingual_concise", render_concise(&diagnostics));
    insta::assert_snapshot!("multilingual_json", render_json(&diagnostics).unwrap());
}

#[test]
fn json_is_a_complete_undecorated_round_trip() {
    let (mut diagnostics, _) = fixture();
    diagnostics[0].fix = Some(Fix {
        message: "Replace source text.".to_owned(),
        applicability: Applicability::Unsafe,
        edits: vec![Edit {
            byte_range: diagnostics[0].byte_range,
            content: "構成".to_owned(),
        }],
    });
    let json = render_json(&diagnostics).unwrap();
    let decoded: Vec<Diagnostic> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, sorted_diagnostics(&diagnostics));
    assert!(!json.contains("Found "));
    assert!(!json.contains("No diagnostics"));
    assert!(!json.contains('\u{1b}'));
    assert!(json.contains("\"applicability\": \"unsafe\""));
    assert_eq!(
        serde_json::to_string(&Applicability::Safe).unwrap(),
        "\"safe\""
    );
}

#[test]
fn shuffled_input_and_related_locations_render_identically() {
    let (mut diagnostics, sources) = fixture();
    let mut same_position = diagnostics[0].clone();
    same_position.message = "Another message at the same position.".to_owned();
    same_position.related.push(RelatedLocation::new(
        "docs/a.md",
        "# A\n",
        Span::new(2, 3),
        "Another definition.",
    ));
    diagnostics.push(same_position.clone());
    same_position.suggestion = "Use the other definition.".to_owned();
    diagnostics.push(same_position);
    let expected_text = render_text(&diagnostics, &sources);
    let expected_concise = render_concise(&diagnostics);
    let expected_json = render_json(&diagnostics).unwrap();
    for iteration in 0..32 {
        diagnostics.rotate_left(1);
        if iteration % 2 == 0 {
            diagnostics.reverse();
        }
        for diagnostic in &mut diagnostics {
            diagnostic.related.reverse();
        }
        assert_eq!(render_text(&diagnostics, &sources), expected_text);
        assert_eq!(render_concise(&diagnostics), expected_concise);
        assert_eq!(render_json(&diagnostics).unwrap(), expected_json);
    }
}

#[test]
fn report_order_prioritizes_path_then_location_then_code() {
    let mut a = diagnostic("中文", "TST002");
    let mut b = diagnostic("text", "TST999");
    let mut c = b.clone();
    let d = diagnostic("設定", "TST003");
    a.filename = "a.md".to_owned();
    b.filename = "a.md".to_owned();
    c.filename = "a.md".to_owned();
    c.code = "TST001".to_owned();
    assert_eq!(
        sorted_diagnostics(&[a.clone(), d.clone(), b.clone(), c.clone()]),
        vec![c, b, a, d]
    );
}

#[test]
fn concise_escapes_newlines_to_keep_one_diagnostic_per_line() {
    let mut diagnostic = diagnostic("text", "TST001");
    diagnostic.filename = "docs/a\nb.md".to_owned();
    diagnostic.message = "First line.\r\nSecond line.".to_owned();
    diagnostic.suggestion = "Step one.\nStep two.".to_owned();
    diagnostic.related.push(RelatedLocation::new(
        "docs/x\ny.md",
        "A",
        Span::new(0, 1),
        "A\nB",
    ));
    let concise = render_concise(&[diagnostic]);
    assert_eq!(concise.lines().count(), 1);
    assert!(concise.contains("docs/a\\nb.md"));
    assert!(concise.contains("First line.\\r\\nSecond line."));
    assert!(concise.contains("Step one.\\nStep two."));
    assert!(concise.contains("Related: docs/x\\ny.md:1:1: A\\nB"));
}

#[test]
fn empty_reports_and_missing_source_are_valid() {
    assert_eq!(render_json(&[]).unwrap(), "[]\n");
    assert_eq!(render_concise(&[]), "");
    assert_eq!(render_text(&[], &BTreeMap::new()), "No diagnostics.\n");
    let report = render_text(&[diagnostic("text", "TST001")], &BTreeMap::new());
    assert!(report.contains("suggestion: Inspect the original Markdown range."));
    assert!(report.ends_with("Found 1 diagnostic.\n"));
}

#[test]
fn multiline_ranges_exclude_end_line_when_ending_at_column_one() {
    let source = "One\r\n二\r\nThree";
    let end = source.find("Three").unwrap();
    let diagnostic = Diagnostic::new(
        "multiline.md",
        source,
        "TST001",
        Span::new(0, end),
        "Two lines.",
        "Review both lines.",
    );
    let report = render_text(
        &[diagnostic],
        &BTreeMap::from([("multiline.md".to_owned(), source.to_owned())]),
    );
    assert!(report.contains("1 | One"));
    assert!(report.contains("2 | 二"));
    assert!(!report.contains("3 | Three"));
}

#[test]
fn constructor_preserves_valid_bytes_and_clamps_invalid_utf8_offsets() {
    let clamped = Diagnostic::new(
        "unicode.md",
        "a中日",
        "TST001",
        Span::new(2, usize::MAX),
        "Unicode range.",
        "Review the range.",
    );
    assert_eq!(clamped.byte_range, Span::new(1, 7));
    assert_eq!(clamped.location.column, 2);
    assert_eq!(clamped.end_location.column, 4);
    let map = SourceMap::new(SOURCE);
    let diagnostic = diagnostic("中文", "TST002");
    assert_eq!(
        map.span_locations(diagnostic.byte_range),
        (diagnostic.location, diagnostic.end_location)
    );
}
