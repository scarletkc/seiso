use std::collections::BTreeSet;

use seiso::diagnostics::{Diagnostic, RelatedLocation, Span};
use seiso::md::parse;
use seiso::rules::suppression::{
    SuppressionResult, SuppressionScope, SuppressionState, apply, inspect,
};

fn codes(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn diagnostic(source: &str, code: &str, needle: &str) -> Diagnostic {
    let start = source.find(needle).unwrap();
    Diagnostic::new(
        "guide.md",
        source,
        code,
        Span::new(start, start + needle.len()),
        "Problem",
        "Change this.",
    )
}

fn run(source: &str, diagnostics: Vec<Diagnostic>, enabled: &[&str]) -> SuppressionResult {
    apply(
        &parse(source).unwrap(),
        "guide.md",
        diagnostics,
        &codes(enabled),
        &BTreeSet::new(),
    )
}

#[test]
fn inspection_reports_declaration_validity_without_assuming_rule_activity() {
    let document = parse("<!-- seiso: allow-file LNK001, PTR001 -- Generated links. -->\n\n<!-- seiso: allow STL999 -- Planned rule. -->\n\n[Link](missing.md)").unwrap();
    let records = inspect(&document, &codes(&["LNK001", "SUP001", "SUP002"]));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].states["LNK001"], SuppressionState::NotEvaluated);
    assert_eq!(records[0].states["PTR001"], SuppressionState::RuleDisabled);
    assert_eq!(records[1].states["STL999"], SuppressionState::Invalid);
    assert!(
        records[1]
            .error
            .as_ref()
            .unwrap()
            .contains("not a known full rule code")
    );
}

#[test]
fn standalone_binds_only_the_next_block() {
    for body in [
        "First paragraph.\n\nSecond paragraph.",
        "- First item.\n- Another item.\n\nSecond paragraph.",
        "| First cell |\n| --- |\n| Another cell |\n\nSecond paragraph.",
        "```text\nFirst example.\n```\n\nSecond paragraph.",
        "# First heading\n\nSecond paragraph.",
    ] {
        let source = format!("<!-- seiso: allow LNK001 -- Published elsewhere. -->\n\n{body}");
        let first = diagnostic(&source, "LNK001", "First");
        let second = diagnostic(&source, "LNK001", "Second");
        let result = run(
            &source,
            vec![first, second.clone()],
            &["LNK001", "SUP001", "SUP002"],
        );
        assert_eq!(result.diagnostics, vec![second], "{source}");
        assert_eq!(
            result.suppressions[0].states["LNK001"],
            SuppressionState::Active { count: 1 }
        );
    }
}

#[test]
fn inline_scopes_cover_paragraph_list_item_and_table_cell() {
    for (source, inside, outside) in [
        (
            "First <!-- seiso: allow LNK001 -- External content. --> paragraph.\n\nSecond paragraph.",
            "First",
            "Second",
        ),
        (
            "- First <!-- seiso: allow LNK001 -- External content. --> item.\n\n  Nested paragraph.\n- Second item.",
            "Nested",
            "Second",
        ),
        (
            "| First <!-- seiso: allow LNK001 -- External content. --> | Second |\n| --- | --- |\n| other | other |",
            "First",
            "Second",
        ),
    ] {
        let first = diagnostic(source, "LNK001", inside);
        let second = diagnostic(source, "LNK001", outside);
        let result = run(
            source,
            vec![first, second.clone()],
            &["LNK001", "SUP001", "SUP002"],
        );
        assert_eq!(result.diagnostics, vec![second], "{source}");
    }
}

#[test]
fn file_scope_accepts_frontmatter_and_leading_comments() {
    let source = "---\nkind: howto\n---\n<!-- License notice. -->\n<!-- seiso: allow-file LNK001 -- Generated links. -->\n\n# Guide\n\nFirst paragraph.\n\nSecond paragraph.";
    let result = run(
        source,
        vec![
            diagnostic(source, "LNK001", "First"),
            diagnostic(source, "LNK001", "Second"),
        ],
        &["LNK001", "SUP001", "SUP002"],
    );
    assert!(result.diagnostics.is_empty());
    assert_eq!(result.suppressions[0].scope, Some(SuppressionScope::File));
    assert_eq!(
        result.suppressions[0].states["LNK001"],
        SuppressionState::Active { count: 2 }
    );
}

#[test]
fn file_scope_must_precede_all_content() {
    for source in [
        "# Guide\n\n<!-- seiso: allow-file LNK001 -- Generated links. -->",
        "Text <!-- seiso: allow-file LNK001 -- Generated links. -->",
        "[link]: target\n\n<!-- seiso: allow-file LNK001 -- Generated links. -->",
    ] {
        let result = run(source, vec![], &["LNK001", "SUP001", "SUP002"]);
        assert_eq!(result.diagnostics.len(), 1, "{source}");
        assert_eq!(result.diagnostics[0].code, "SUP001");
        assert!(result.suppressions[0].error.is_some());
    }
}

#[test]
fn malformed_declarations_never_suppress_even_when_sup001_is_disabled() {
    for declaration in [
        "allow LNK -- Prefixes are rejected.",
        "allow LNK999 -- Unknown code.",
        "allow lnk001 -- Wrong case.",
        "allow LNK001 --   ",
        "allow LNK001 missing reason separator",
        "allow LNK001, -- Empty code.",
        "allow LNK001 LNK002 -- Missing comma.",
        "allow LNK001, LNK001 -- Duplicate code.",
        "ignore LNK001 -- Wrong command.",
        "allow",
        "allow LNK001 -- Multiple\nlines.",
    ] {
        let source = format!("<!-- seiso: {declaration} -->\n\nTarget.");
        let expected = diagnostic(&source, "LNK001", "Target");
        let enabled = run(
            &source,
            vec![expected.clone()],
            &["LNK001", "SUP001", "SUP002"],
        );
        assert_eq!(enabled.diagnostics.len(), 2, "{source}");
        assert!(enabled.diagnostics.iter().any(|item| item.code == "SUP001"));
        let disabled = run(&source, vec![expected.clone()], &["LNK001", "SUP002"]);
        assert_eq!(disabled.diagnostics, vec![expected], "{source}");
    }
}

#[test]
fn code_examples_and_unrelated_comments_are_inert() {
    let source = "```markdown\n<!-- seiso: allow LNK001 -- Example. -->\n```\n\n`<!-- seiso: allow LNK001 -- Example. -->`\n\n    <!-- seiso: allow LNK001 -- Example. -->\n\n<!-- Ordinary comment. -->\n\nTarget.";
    let expected = diagnostic(source, "LNK001", "Target");
    let result = run(
        source,
        vec![expected.clone()],
        &["LNK001", "SUP001", "SUP002"],
    );
    assert_eq!(result.diagnostics, vec![expected]);
    assert!(result.suppressions.is_empty());
}

#[test]
fn block_scope_takes_credit_before_file_scope() {
    let source = "<!-- seiso: allow-file LNK001 -- Global allowance. -->\n\n<!-- seiso: allow LNK001 -- Local allowance. -->\n\nTarget.";
    let result = run(
        source,
        vec![diagnostic(source, "LNK001", "Target")],
        &["LNK001", "SUP001", "SUP002"],
    );
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, "SUP002");
    assert_eq!(
        result.diagnostics[0].byte_range,
        result.suppressions[0].span
    );
    assert_eq!(
        result.suppressions[0].states["LNK001"],
        SuppressionState::Stale
    );
    assert_eq!(
        result.suppressions[1].states["LNK001"],
        SuppressionState::Active { count: 1 }
    );
}

#[test]
fn only_primary_location_can_match() {
    let source = "<!-- seiso: allow LNK001 -- Local allowance. -->\n\nFirst.\n\nSecond.";
    let mut item = diagnostic(source, "LNK001", "Second");
    let first = source.find("First").unwrap();
    item.related.push(RelatedLocation::new(
        "guide.md",
        source,
        Span::new(first, first + 5),
        "Related",
    ));
    let result = run(source, vec![item.clone()], &["LNK001", "SUP002"]);
    assert_eq!(result.diagnostics[0], item);
    assert_eq!(result.diagnostics[1].code, "SUP002");
    let mut foreign = diagnostic(source, "LNK001", "First");
    foreign.filename = "other.md".into();
    let result = run(source, vec![foreign.clone()], &["LNK001"]);
    assert_eq!(result.diagnostics, vec![foreign]);
}

#[test]
fn state_is_per_code_and_disabled_or_incomplete_rules_are_not_stale() {
    let source =
        "<!-- seiso: allow LNK001, LNK002, STL001, PTR001 -- External conventions. -->\n\nTarget.";
    let result = apply(
        &parse(source).unwrap(),
        "guide.md",
        vec![],
        &codes(&["LNK001", "STL001", "SUP001", "SUP002"]),
        &codes(&["STL001"]),
    );
    let states = &result.suppressions[0].states;
    assert_eq!(states["LNK001"], SuppressionState::Stale);
    assert_eq!(states["LNK002"], SuppressionState::RuleDisabled);
    assert_eq!(states["PTR001"], SuppressionState::RuleDisabled);
    assert_eq!(states["STL001"], SuppressionState::Incomplete { count: 0 });
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.contains("LNK001"));
    assert!(result.diagnostics[0].fix.is_none());
}

#[test]
fn incomplete_rules_can_suppress_known_diagnostics_without_claiming_completion() {
    let source = "<!-- seiso: allow LNK001 -- External convention. -->\n\nTarget.";
    let result = apply(
        &parse(source).unwrap(),
        "guide.md",
        vec![diagnostic(source, "LNK001", "Target")],
        &codes(&["LNK001", "SUP002"]),
        &codes(&["LNK001"]),
    );
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result.suppressions[0].states["LNK001"],
        SuppressionState::Incomplete { count: 1 }
    );
}

#[test]
fn missing_or_unsupported_following_block_is_invalid() {
    for ending in ["", "---\n\nLater.", "> Quoted paragraph."] {
        let source = format!("<!-- seiso: allow LNK001 -- Local allowance. -->\n\n{ending}");
        let result = run(&source, vec![], &["SUP001"]);
        assert_eq!(result.diagnostics.len(), 1, "{source}");
        assert_eq!(result.diagnostics[0].code, "SUP001");
    }
    let result = run(
        "# Heading <!-- seiso: allow LNK001 -- Local allowance. -->",
        vec![],
        &["SUP001"],
    );
    assert_eq!(result.diagnostics[0].code, "SUP001");
}

#[test]
fn nested_standalone_scope_stays_inside_its_list_item() {
    let source =
        "- Intro.\n\n  <!-- seiso: allow LNK001 -- Nested exception. -->\n\n  Target.\n- Outside.";
    let outside = diagnostic(source, "LNK001", "Outside");
    let result = run(
        source,
        vec![diagnostic(source, "LNK001", "Target"), outside.clone()],
        &["LNK001", "SUP001", "SUP002"],
    );
    assert_eq!(result.diagnostics, vec![outside]);
}

#[test]
fn meta_suppressions_do_not_recurse_or_hide_themselves() {
    let source = "<!-- seiso: allow SUP002 -- Preserve a local exception. -->\n\nTarget.";
    let result = run(source, vec![], &["SUP002"]);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, "SUP002");
    assert_eq!(
        result.suppressions[0].states["SUP002"],
        SuppressionState::Stale
    );

    let source = "<!-- seiso: allow-file SUP001 -- Legacy syntax is quoted literally. -->\n\n<!-- seiso: allow UNKNOWN -- Invalid. -->\n\nTarget.";
    let result = run(source, vec![], &["SUP001", "SUP002"]);
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result.suppressions[0].states["SUP001"],
        SuppressionState::Active { count: 1 }
    );

    let source = "<!-- seiso: allow-file SUP002 -- Retained release exemptions. -->\n\n<!-- seiso: allow LNK001 -- Generated target. -->\n\nTarget.";
    let result = run(source, vec![], &["LNK001", "SUP002"]);
    assert!(result.diagnostics.is_empty());
    assert_eq!(
        result.suppressions[0].states["SUP002"],
        SuppressionState::Active { count: 1 }
    );
    assert_eq!(
        result.suppressions[1].states["LNK001"],
        SuppressionState::Stale
    );

    let source = "<!-- seiso: allow-file SUP002 -- First exemption. -->\n<!-- seiso: allow-file SUP002 -- Second exemption. -->\n\nTarget.";
    let result = run(source, vec![], &["SUP002"]);
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|item| item.code == "SUP002"));

    let source = "<!-- seiso: allow-file LNK001, SUP002 -- Shared exemption. -->\n\nTarget.";
    let result = run(source, vec![], &["LNK001", "SUP002"]);
    assert_eq!(result.diagnostics.len(), 2);
}

#[test]
fn serialized_policy_preserves_reasons_scope_and_per_code_states() {
    let source = "<!-- seiso: allow-file LNK001, LNK002 -- 公開時に生成。 -->\n\nText.";
    let result = run(source, vec![], &["LNK001"]);
    let value = serde_json::to_value(&result.suppressions[0]).unwrap();
    assert_eq!(value["reason"], "公開時に生成。");
    assert_eq!(value["scope"]["kind"], "file");
    assert_eq!(value["states"]["LNK001"]["state"], "stale");
    assert_eq!(value["states"]["LNK002"]["state"], "rule_disabled");
}
