//! Constructed protocol cases; these are not natural precision samples.
mod common;
use common::{CheckContext, check};

use std::collections::BTreeSet;
use std::path::Path;

use seiso::config::{CliOverrides, Config};
use seiso::diagnostics::{Applicability, Diagnostic, Location, Span};
use seiso::rules::CheckResult;
use seiso::rules::suppression::{SuppressionScope, SuppressionState, apply};

fn evaluate(source: &str, configuration: &str) -> CheckResult {
    let root = Path::new("/contract");
    let document = seiso::md::parse(source).unwrap();
    let config = Config::parse(configuration, root).unwrap();
    let context = CheckContext {
        document: &document,
        filename: "docs/guide.md",
        path: &root.join("docs/guide.md"),
        workspace_root: root,
        config: &config,
        overrides: &CliOverrides {
            preview: true,
            ..CliOverrides::default()
        },
    };
    let first = check(&context).unwrap();
    let second = check(&context).unwrap();
    assert!(first.errors.is_empty());
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap(),
        "repeated protocol evaluation must preserve the complete result"
    );
    first
}

fn rule_codes(result: &CheckResult) -> Vec<&str> {
    result
        .diagnostics
        .iter()
        .map(|item| item.code.as_str())
        .collect()
}

#[test]
fn kind_vocabulary_and_invalid_yaml_have_distinct_outcomes() {
    let config = "[lint]\nselect=['KND001','KND002']";
    for kind in [
        "readme",
        "howto",
        "reference",
        "runbook",
        "adr",
        "plan",
        "changelog",
    ] {
        let source = format!("---\nkind: {kind}\n---\n\n# 文档 / Guide / ガイド\n");
        let result = evaluate(&source, config);
        assert!(result.diagnostics.is_empty(), "{kind}");
        assert_eq!(
            result.kind.value.map(seiso::rules::Kind::as_str),
            Some(kind)
        );
    }
    for kind in ["guide", "generated", "HOWTO", "''", "手順"] {
        let source = format!("---\nkind: {kind}\n---\n\n# Guide\n");
        let result = evaluate(&source, config);
        assert_eq!(rule_codes(&result), ["KND002"], "{kind}");
        assert_eq!(result.kind.value, None);
        let expected_end = source.find("\n---\n").unwrap() + 4;
        assert_eq!(result.diagnostics[0].byte_range, Span::new(0, expected_end));
        assert!(result.diagnostics[0].fix.is_none());
    }
    for declaration in ["kind: [", "kind: 123", "kind: true", "kind: []"] {
        let result = evaluate(&format!("---\n{declaration}\n---\n"), config);
        assert_eq!(rule_codes(&result), ["KND001"], "{declaration}");
    }
}

#[test]
fn kind_examples_and_unrelated_comments_do_not_declare_a_kind() {
    let source =
        "```yaml\n---\nkind: generated\n---\n```\n\n`kind: guide`\n\n<!-- kind: guide -->\n";
    let result = evaluate(source, "[lint]\nselect=['KND001','KND002']");
    assert_eq!(rule_codes(&result), ["KND001"]);
    assert_eq!(result.kind.value, None);
}

fn assert_exact_comment_location(diagnostic: &Diagnostic, source: &str, comment: &str) {
    let start = source.find(comment).unwrap();
    let prefix = &source[..start];
    let column = prefix.rsplit('\n').next().unwrap().chars().count() + 1;
    let row = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    assert_eq!(
        diagnostic.byte_range,
        Span::new(start, start + comment.len())
    );
    assert_eq!(diagnostic.location, Location { row, column });
    assert_eq!(
        diagnostic.end_location,
        Location {
            row,
            column: column + comment.chars().count()
        }
    );
    if diagnostic.code == "SUP002" {
        assert_eq!(
            diagnostic.fix.as_ref().unwrap().applicability,
            Applicability::Safe
        );
    } else {
        assert!(diagnostic.fix.is_none());
    }
}

#[test]
fn suppression_diagnostics_preserve_unicode_columns_and_line_endings() {
    let config =
        "[[kinds]]\npath='docs/**'\nkind='howto'\n[lint]\nselect=['PTR003','SUP001','SUP002']";
    for newline in ["\n", "\r\n"] {
        for (comment, expected) in [
            ("<!-- seiso: allow PTR -- 理由 / 理由。 -->", "SUP001"),
            ("<!-- seiso: allow PTR003 -- 理由 / 理由。 -->", "SUP002"),
        ] {
            let source = format!(
                "# 文档{newline}{newline}中文、日本語、😀 {comment} No navigation here.{newline}"
            );
            let result = evaluate(&source, config);
            assert_eq!(rule_codes(&result), [expected]);
            assert_exact_comment_location(&result.diagnostics[0], &source, comment);
        }
    }
}

#[test]
fn suppression_completion_uses_effective_engine_policy() {
    let source = "<!-- seiso: allow STL001 -- 保留例外。 -->\n\nNo snapshot here.\n";
    for config in [
        "[[kinds]]\npath='docs/**'\nkind='howto'\n[lint]\nselect=['SUP002']",
        "[[kinds]]\npath='docs/**'\nkind='howto'\n[lint]\nselect=['STL001','SUP002']\n[lint.per-file-ignores]\n'docs/**'=['STL001']",
        "[lint]\nselect=['STL001','SUP002']",
    ] {
        let result = evaluate(source, config);
        assert!(result.diagnostics.is_empty());
        assert_eq!(
            result.suppressions[0].states["STL001"],
            SuppressionState::RuleDisabled
        );
    }
    let result = evaluate(
        source,
        "[[kinds]]\npath='docs/**'\nkind='howto'\n[lint]\nselect=['STL001','SUP002']",
    );
    assert_eq!(rule_codes(&result), ["SUP002"]);
    assert_eq!(
        result.suppressions[0].states["STL001"],
        SuppressionState::Stale
    );

    let result = evaluate(
        source,
        "[[kinds]]\npath='docs/**'\nkind='generated'\n[lint]\nselect=['STL001','SUP002']",
    );
    assert!(result.diagnostics.is_empty());
    assert!(result.suppressions.is_empty());
    assert!(result.enabled_rules.is_empty());
}

#[test]
fn overlapping_block_scopes_choose_smallest_then_earliest() {
    for (source, winner) in [
        (
            "- Parent <!-- seiso: allow PTR003 -- Parent exemption. -->\n  - Child <!-- seiso: allow PTR003 -- Child exemption. --> Target.\n",
            1,
        ),
        (
            "Target <!-- seiso: allow PTR003 -- First exemption. --> text <!-- seiso: allow PTR003 -- Second exemption. -->.\n",
            0,
        ),
    ] {
        let document = seiso::md::parse(source).unwrap();
        let start = source.find("Target").unwrap();
        let diagnostic = Diagnostic::new(
            "docs/guide.md",
            source,
            "PTR003",
            Span::new(start, start + "Target".len()),
            "Constructed scope probe",
            "Inspect the selected scope.",
        );
        let enabled = BTreeSet::from(["PTR003".into(), "SUP001".into(), "SUP002".into()]);
        let first = apply(
            &document,
            "docs/guide.md",
            vec![diagnostic.clone()],
            &enabled,
            &BTreeSet::new(),
        );
        let second = apply(
            &document,
            "docs/guide.md",
            vec![diagnostic],
            &enabled,
            &BTreeSet::new(),
        );
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.suppressions.len(), 2);
        let spans: Vec<_> = first
            .suppressions
            .iter()
            .map(|record| match record.scope {
                Some(SuppressionScope::Block { span }) => span,
                _ => panic!("both declarations must have block scope"),
            })
            .collect();
        if winner == 0 {
            assert_eq!(spans[0], spans[1]);
        } else {
            assert!(spans[0].start <= spans[1].start && spans[1].end <= spans[0].end);
            assert!(spans[1].len() < spans[0].len());
        }
        assert!(first.suppressions[0].span.start < first.suppressions[1].span.start);
        assert_eq!(
            first.suppressions[winner].states["PTR003"],
            SuppressionState::Active { count: 1 }
        );
        assert_eq!(
            first.suppressions[1 - winner].states["PTR003"],
            SuppressionState::Stale
        );
        assert_eq!(first.diagnostics.len(), 1);
        assert_eq!(first.diagnostics[0].code, "SUP002");
        assert_eq!(
            first.diagnostics[0].byte_range,
            first.suppressions[1 - winner].span
        );
        assert!(first.diagnostics[0].fix.is_none());
    }
}

#[test]
fn equal_file_meta_suppressions_receive_no_circular_credit() {
    let source = "<!-- seiso: allow-file SUP002 -- First exemption. -->\n<!-- seiso: allow-file SUP002 -- Second exemption. -->\n\nText.\n";
    let result = evaluate(source, "[lint]\nselect=['SUP002']");
    assert_eq!(rule_codes(&result), ["SUP002", "SUP002"]);
    for (record, diagnostic) in result.suppressions.iter().zip(&result.diagnostics) {
        assert_eq!(record.scope, Some(SuppressionScope::File));
        assert_eq!(record.states["SUP002"], SuppressionState::Stale);
        assert_eq!(record.span, diagnostic.byte_range);
        assert_eq!(
            diagnostic.fix.as_ref().unwrap().applicability,
            Applicability::Safe
        );
    }
}
