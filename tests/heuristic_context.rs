use seiso::config::{CliOverrides, Config};
use seiso::md::parse;
use seiso::rules::{CheckContext, CheckResult, check};
use seiso::sections::{SectionType, classify};

fn evaluate(source: &str, code: &str, kind: &str) -> CheckResult {
    let root = std::env::current_dir().unwrap();
    let document = parse(source).unwrap();
    let config = Config::parse(&format!("preview=true\n[[kinds]]\npath='**/*.md'\nkind='{kind}'\n[lint]\nselect=['{code}','SUP002']"), &root).unwrap();
    check(&CheckContext {
        document: &document,
        filename: "guide.md",
        path: &root.join("guide.md"),
        workspace_root: &root,
        config: &config,
        overrides: &CliOverrides::default(),
    })
    .unwrap()
}

#[test]
fn missing_main_flow_preserves_ordering_suppressions() {
    for code in ["RAT001", "ORD001", "ORD002"] {
        let source = format!(
            "<!-- seiso: allow-file {code} -- The procedure is not recognized. -->\n\nThe next action is to launch the application."
        );
        let result = evaluate(&source, code, "howto");
        assert!(result.diagnostics.is_empty(), "{code}");
        assert!(matches!(
            result.suppressions[0].states[code],
            seiso::rules::suppression::SuppressionState::Incomplete { count: 0 }
        ));

        let completed = evaluate(&format!("{source}\n\nRun `app start`."), code, "howto");
        assert_eq!(completed.diagnostics[0].code, "SUP002");
        assert!(completed.diagnostics[0].fix.is_some());
    }
}

#[test]
fn classifier_and_mixed_role_rule_share_decision_headings() {
    for heading in [
        "Why we choose a queue",
        "Why we chose a queue",
        "Why we use a queue",
    ] {
        let source = format!(
            "## {heading}\n\nWorkers share a durable message buffer.\n\nEach request has an independent lifetime."
        );
        assert_eq!(
            classify(&parse(&source).unwrap())[1].section_type,
            SectionType::Rationale
        );
        assert_eq!(
            evaluate(&source, "MIX001", "howto").diagnostics[0].code,
            "MIX001"
        );
    }
    assert!(
        evaluate(
            "## Why use a queue\n\nWorkers share messages.\n\nRequests are independent.",
            "MIX001",
            "howto"
        )
        .diagnostics
        .is_empty()
    );
}

#[test]
fn quoted_decisions_do_not_become_document_rationale() {
    let arguments = [
        "We chose this database because our team needed to compare several persistence models and their operational costs",
        "We chose a different deployment design after a long discussion about maintenance trade-offs and operational overhead",
    ];
    let quoted = format!(
        "## Logs\n\nThe log contains \"{}\".\n\nAnother log contains \"{}\".",
        arguments[0], arguments[1]
    );
    assert_ne!(
        classify(&parse(&quoted).unwrap())[1].section_type,
        SectionType::Rationale
    );
    for code in ["RAT001", "MIX001"] {
        assert!(
            evaluate(
                &format!("{quoted}\n\n## Start\n\nRun `app start`."),
                code,
                "howto"
            )
            .diagnostics
            .is_empty()
        );
        let asserted = format!(
            "## Choice\n\n{}.\n\n{}.\n\n## Start\n\nRun `app start`.",
            arguments[0], arguments[1]
        );
        assert_eq!(evaluate(&asserted, code, "howto").diagnostics[0].code, code);
    }
}

#[test]
fn version_constraints_and_examples_do_not_hide_independent_snapshots() {
    for source in [
        "Install Node.js v18.0.0 or higher.",
        "Versions prior to v1.14.0 store history differently.",
        "Use a version such as `1.2.3` or `v1.2.3`.",
        "バージョン 3.2 より前では利用できません。",
    ] {
        assert!(
            evaluate(source, "STL004", "howto").diagnostics.is_empty(),
            "{source}"
        );
    }
    assert_eq!(
        evaluate(
            "Versions prior to v1.14.0 differ. The deployed version is v1.15.0.",
            "STL004",
            "howto"
        )
        .diagnostics
        .len(),
        1
    );
    assert_eq!(
        evaluate("The version is v1.14.0.", "STL004", "howto")
            .diagnostics
            .len(),
        1
    );
}

#[test]
fn quoted_examples_keep_offsets_and_do_not_mask_following_assertions() {
    for source in [
        "Discuss evaluative phrases (is faster, is better) during review.",
        "评价性形容词（推荐、更快、最佳）需要依据。",
    ] {
        assert!(
            evaluate(source, "EVD001", "howto").diagnostics.is_empty(),
            "{source}"
        );
    }
    for (source, code, expected) in [
        (
            "The phrase “is faster” is a claim. This implementation is faster.",
            "EVD001",
            "is faster",
        ),
        ("示例：“更快”。这个实现更快。", "EVD001", "更快"),
        (
            "The example says \"is deployed\". The service is deployed to production.",
            "STL002",
            "is deployed",
        ),
        (
            "A table quotes “this page demonstrates”. I have implemented the editor.",
            "VOX003",
            "I have implemented",
        ),
    ] {
        let result = evaluate(source, code, "howto");
        assert_eq!(result.diagnostics.len(), 1, "{source}");
        let span = result.diagnostics[0].byte_range;
        assert_eq!(&source[span.start..span.end], expected);
        assert!(span.start >= source.find('.').or_else(|| source.find('。')).unwrap());
    }
    assert!(evaluate("## Example usage\n\n### Assistant response\n\nThe service is deployed.\n\n## Actual state\n\nCheck the dashboard.", "STL002", "howto").diagnostics.is_empty());
}

#[test]
fn recommendations_can_have_operational_evidence_but_comparative_claims_still_need_support() {
    for source in [
        "We recommend creating a kernel for the project.\n\n```sh\nuv run ipython kernel install\n```",
        "We recommend PascalCase for consistency and to distinguish native custom elements.",
        "パスカルケースを推奨します。これはネイティブ要素との区別に役立ちます。",
        "This implementation is faster.\n\n| Variant | Duration |\n| --- | --- |\n| A | 12 ms |\n| B | 7 ms |",
    ] {
        assert!(
            evaluate(source, "EVD001", "howto").diagnostics.is_empty(),
            "{source}"
        );
    }
    for source in [
        "We recommend this package over the alternative.",
        "This package is faster because it uses a cache.",
        "This implementation is faster.\n\n```sh\napp start\n```",
    ] {
        assert_eq!(
            evaluate(source, "EVD001", "howto").diagnostics.len(),
            1,
            "{source}"
        );
    }
}

#[test]
fn main_flow_includes_configuration_and_inline_instructions_but_excludes_console_output() {
    let preamble = "Background text.\n".repeat(31);
    for start in [
        "Create `auth.json` with this configuration:\n\n```json\n{}\n```",
        "Run `uv add torch`.",
        "Copy this workflow:\n\n```yaml\njobs: {}\n```",
    ] {
        assert!(
            evaluate(
                &format!("{start}\n\n{preamble}\n\n1. Run `app start`."),
                "ORD001",
                "howto"
            )
            .diagnostics
            .is_empty()
        );
    }
    for ending in [
        "1. Run `app start`.",
        "```bash\napp start\n```",
        "Run `app start`.",
    ] {
        assert_eq!(
            evaluate(&format!("{preamble}\n{ending}"), "ORD001", "howto")
                .diagnostics
                .len(),
            1
        );
    }
    assert!(evaluate(&format!("{preamble}\nThe following error is printed:\n\n```console\nerror: missing field\n```"), "ORD001", "howto").diagnostics.is_empty());
    assert!(
        evaluate(
            &format!("{preamble}\n1. Run `app start`."),
            "ORD001",
            "reference"
        )
        .diagnostics
        .is_empty()
    );
    let document = parse("# Compiler example\n\nThe following error is printed:\n\n```console\n$ cargo run\nerror: bad type\n```").unwrap();
    assert_ne!(classify(&document)[1].section_type, SectionType::Steps);
}

#[test]
fn opaque_containers_preserve_undetermined_suppressions_without_disabling_known_rules() {
    let source = "<!-- seiso: allow-file ORD001 -- Flow is rendered by the site. -->\n\n<Tabs>\n  ```sh\n  app start\n  ```\n</Tabs>\n\nBackground text.\n\n1. Run `app stop`.";
    let result = evaluate(source, "ORD001", "howto");
    assert!(result.diagnostics.is_empty());
    assert!(matches!(
        result.suppressions[0].states["ORD001"],
        seiso::rules::suppression::SuppressionState::Incomplete { count: 0 }
    ));
    let plain = source.replace(
        "<Tabs>\n  ```sh\n  app start\n  ```\n</Tabs>",
        "```sh\napp start\n```",
    );
    assert_eq!(
        evaluate(&plain, "ORD001", "howto").diagnostics[0].code,
        "SUP002"
    );
    assert_eq!(
        evaluate(
            &format!("{source}\n\nThe version is v2.3.1."),
            "STL004",
            "howto"
        )
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "STL004")
        .count(),
        1
    );
}

#[test]
fn section_roles_need_responsibility_evidence_not_incidental_words() {
    for (source, expected) in [
        (
            "## useModel() {#usemodel}\n\nDefines model bindings.",
            SectionType::Reference,
        ),
        (
            "## Description\n\nSkills package instructions for a task.",
            SectionType::Background,
        ),
        (
            "## Messages not formatting correctly\n\nComplex formatting may be simplified.",
            SectionType::Troubleshooting,
        ),
        (
            "## 例外処理\n\nJavaScript の例外について学びます。",
            SectionType::Background,
        ),
        (
            "## Returning values\n\nThis chapter describes expressions and how function return values behave in the language. It explains the execution model in some detail.\n\nThe examples illustrate return expressions and implicit values rather than a sequence of installation tasks for a reader.",
            SectionType::Background,
        ),
    ] {
        assert_eq!(
            classify(&parse(source).unwrap())[1].section_type,
            expected,
            "{source}"
        );
    }
    assert_ne!(classify(&parse("## Full-stack visibility\n\nSee the [alternative clients guide](https://example.org/clients).").unwrap())[1].section_type, SectionType::Rationale);
    assert_ne!(
        classify(
            &parse("## Behavior\n\n1. 要素内の空白を圧縮します。\n2. 改行を除去します。").unwrap()
        )[1]
        .section_type,
        SectionType::Steps
    );
    assert_eq!(classify(&parse("## Overview\n\nRead the concept pages:\n\n- [Projects](projects.md)\n- [Tools](tools.md)").unwrap())[1].section_type, SectionType::Other);
}

#[test]
fn orientation_is_not_a_project_decision_and_configuration_can_start_inline() {
    assert_eq!(
        evaluate(
            &format!(
                "{}\n## Example usage\n\n```sh\napp start\n```",
                "Background.\n".repeat(31)
            ),
            "ORD001",
            "howto"
        )
        .diagnostics
        .len(),
        1
    );
    assert!(
        evaluate(
            &format!(
                "{}\n## Example output\n\n```console\n$ app start\nStarted\n```",
                "Background.\n".repeat(31)
            ),
            "ORD001",
            "howto"
        )
        .diagnostics
        .is_empty()
    );
    assert!(evaluate("## Why Use a Cache?\n\nA cache can store repeated results.\n\nIt also reduces repeated work for callers.", "MIX001", "howto").diagnostics.is_empty());
    assert_eq!(evaluate("## Why We Chose a Cache\n\nWe chose this cache for shared state.\n\nOur decision followed a review of the alternatives.", "MIX001", "howto").diagnostics.len(), 1);
    assert!(evaluate(&format!("To start, consider this configuration generated by `app init`.\n\n```toml\nport = 8080\n```\n\n{}\n1. Run `app start`.", "Background.\n".repeat(31)), "ORD001", "howto").diagnostics.is_empty());
    assert_eq!(
        classify(&parse("## 知識チェック\n\n正しい答えを示してください。\n\n1. A\n2. B").unwrap())
            [1]
        .section_type,
        SectionType::Other
    );
}

#[test]
fn assertions_require_state_relations_and_ambiguous_candidates_preserve_suppressions() {
    for source in [
        "New in version 1.5.",
        "The config object was new in MkDocs version 1.0.",
        "As of version 0.17, support has been added.",
        "The editor announced support in the 2024.4 release.",
        "Release title `Version 0.13.3`.",
    ] {
        assert!(
            evaluate(source, "STL004", "howto").diagnostics.is_empty(),
            "{source}"
        );
    }
    for source in [
        "Version:2.3.1",
        "The service runs version 2.3.1.",
        "The release is v2.3.1.",
        "版本为 2.3.1。",
        "バージョンは 2.3.1 です。",
    ] {
        assert_eq!(
            evaluate(source, "STL004", "howto").diagnostics.len(),
            1,
            "{source}"
        );
    }
    for (code, source) in [
        ("EVD001", "This naming convention is recommended."),
        ("STL002", "Site files are deployed to the main branch."),
    ] {
        let source = format!(
            "<!-- seiso: allow {code} -- Contract interpretation requires review. -->\n{source}"
        );
        let result = evaluate(&source, code, "howto");
        assert!(result.diagnostics.is_empty());
        assert!(matches!(
            result.suppressions[0].states[code],
            seiso::rules::suppression::SuppressionState::Incomplete { count: 0 }
        ));
    }
    assert_eq!(
        evaluate("The service has been deployed.", "STL002", "howto")
            .diagnostics
            .len(),
        1
    );
    assert_eq!(
        evaluate(
            "The service is currently deployed to production.",
            "STL002",
            "howto"
        )
        .diagnostics
        .len(),
        1
    );
    assert_eq!(
        evaluate(
            "The service currently is deployed to production.",
            "STL002",
            "howto"
        )
        .diagnostics
        .len(),
        1
    );
    let preamble = "Background.\n".repeat(31);
    let source = format!(
        "<!-- seiso: allow-file ORD001 -- Earlier configuration may establish the main flow. -->\n\n{preamble}\n```html\n<html></html>\n```\n\n1. Run `app start`."
    );
    let result = evaluate(&source, "ORD001", "howto");
    assert!(result.diagnostics.is_empty());
    assert!(matches!(
        result.suppressions[0].states["ORD001"],
        seiso::rules::suppression::SuppressionState::Incomplete { count: 0 }
    ));
}
