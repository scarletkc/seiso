mod common;
use common::{CheckContext, check};
use seiso::config::{CliOverrides, Config};
use seiso::diagnostics::Diagnostic;

use seiso::sections::{SectionType, classify};

fn evaluate(source: &str, code: &str, kind: &str, extra: &str, preview: bool) -> Vec<Diagnostic> {
    let root = std::env::current_dir().unwrap();
    let config = Config::parse(&format!("[[kinds]]\npath='**/*.md'\nkind='{kind}'\n[lint]\nselect=['{code}','SUP001','SUP002']\n{extra}"), &root).unwrap();
    check(&CheckContext {
        document: &seiso::md::parse(source).unwrap(),
        filename: "guide.md",
        path: &root.join("guide.md"),
        workspace_root: &root,
        config: &config,
        overrides: &CliOverrides {
            preview,
            ..CliOverrides::default()
        },
    })
    .unwrap()
    .diagnostics
}

fn codes(source: &str, code: &str) -> Vec<Diagnostic> {
    evaluate(source, code, "howto", "", true)
}

#[test]
fn classifications_cover_direct_sections_without_inheriting_children() {
    let source = "# Guide\n\nIntro.\n\n## Background\n\nContext.\n\n## Procedure\n\n1. Run it.\n\n## Parameters\n\n| Name | Value |\n| --- | --- |\n| x | 1 |\n\n## Rationale\n\nA decision.\n\n## FAQ\n\nA question.\n";
    let document = seiso::md::parse(source).unwrap();
    let annotations = classify(&document);
    assert_eq!(
        annotations
            .iter()
            .map(|item| item.section_type)
            .collect::<Vec<_>>(),
        [
            SectionType::Other,
            SectionType::Other,
            SectionType::Background,
            SectionType::Steps,
            SectionType::Reference,
            SectionType::Rationale,
            SectionType::Troubleshooting
        ]
    );
    for item in annotations {
        assert!(
            source
                .get(item.content_span.start..item.content_span.end)
                .is_some()
        );
        for evidence in item.evidence {
            assert!(source.get(evidence.span.start..evidence.span.end).is_some());
        }
    }
    let document =
        seiso::md::parse("# Guide\n\n> 1. Run it.\n\n## `Rationale`\n\nNo argument.\n").unwrap();
    assert!(
        classify(&document)
            .iter()
            .all(|item| item.section_type == SectionType::Other)
    );
}

#[test]
fn deployment_conditions_code_quotes_and_sentence_boundaries() {
    for source in [
        "If the service is deployed, test it.",
        "Check whether the service is live.",
        "Once the service is deployed, connect.",
        "The service will be deployed.",
        "`is deployed`",
        "```text\nis deployed\n```",
        "> The service is deployed.",
        "The URL is https://example.org/is-deployed.",
    ] {
        assert!(codes(source, "STL002").is_empty(), "{source}");
    }
    let source = "If needed, restart. The service **is deployed** to production.";
    let result = codes(source, "STL002");
    assert_eq!(result.len(), 1);
    assert_eq!(
        &source[result[0].byte_range.start..result[0].byte_range.end],
        "is deployed"
    );
    for (source, expected) in [
        ("服务已经部署到生产环境。", "已经部署"),
        ("サービスはデプロイ済みです。", "デプロイ済み"),
    ] {
        let result = codes(source, "STL002");
        assert_eq!(result.len(), 1, "{source}");
        assert_eq!(
            &source[result[0].byte_range.start..result[0].byte_range.end],
            expected
        );
    }
}

#[test]
fn versions_preserve_constraints_history_paths_and_urls() {
    for source in [
        "Version >= 2.3.1.",
        "Version 2.3.1 or later.",
        "Version 2.3+.",
        "Requires version `2.3.1`.",
        "Added in version 2.3.1.",
        "Compatible with version 2.3.1.",
        "For example, version 2.3.1.",
        "Version https://example.org/v2.3.1.",
        "Version `docs/v2.3.1.md`.",
        "Model gpt-v2.3.1.",
        "An address 127.0.0.1.",
        "A ratio of 2.3.",
        "Version `~2.3.1`.",
    ] {
        assert!(codes(source, "STL004").is_empty(), "{source}");
    }
    for source in [
        "The version is `2.3.1`.",
        "The release is v2.3.1.",
        "版本为 2.3.1。",
        "バージョンは 2.3.1 です。",
    ] {
        assert_eq!(codes(source, "STL004").len(), 1, "{source}");
    }
    assert!(
        evaluate(
            "Version 2.3.1 on supported systems.",
            "STL004",
            "howto",
            "[lint.lexicon.en]\nextend-constraint-markers=['supported systems']",
            true
        )
        .is_empty()
    );
}

#[test]
fn a_value_gets_one_staleness_diagnostic_from_the_most_specific_rule() {
    let reported = |source: &str, ignore: &str| {
        evaluate(source, "STL", "howto", ignore, true)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>()
    };
    for (source, expected) in [
        ("目前版本是 v1.2.3。", "STL004"),
        ("The latest release is v2.0.1.", "STL004"),
        ("Production currently runs commit abc1234f.", "STL003"),
        ("Currently uses v1.2.3.", "STL001"),
        ("目前有 12 个迁移。", "STL001"),
    ] {
        assert_eq!(reported(source, ""), [expected], "{source}");
    }
    for (source, ignore) in [
        ("The latest release is v2.0.1.", "ignore=['STL004']"),
        (
            "Production currently runs commit abc1234f.",
            "ignore=['STL003']",
        ),
    ] {
        assert_eq!(reported(source, ignore), ["STL001"], "{source}");
    }
}

#[test]
fn ownership_notices_are_not_production_narration() {
    for source in [
        "This document was generated by `make docs`; do not edit it by hand.",
        "本文档由社区维护，欢迎提交修改。",
        "このドキュメントは `make docs` で生成されます。",
    ] {
        assert!(codes(source, "VOX003").is_empty(), "{source}");
    }
    assert_eq!(
        codes("I have implemented the retry loop.", "VOX003").len(),
        1
    );
}

#[test]
fn claims_need_local_evidence_not_a_distant_or_nested_link() {
    for source in [
        "This is faster [source](https://example.org/result).",
        "This is faster.\n\nMeasured 10 ms.",
        "See [results](https://example.org/result).\n\nThis is faster.",
        "If this is faster, use it.",
        "`This is faster`.",
        "> This is faster.",
        "This is faster in the benchmark.",
    ] {
        assert!(codes(source, "EVD001").is_empty(), "{source}");
    }
    for source in [
        "This is faster.",
        "This is faster.\n\nUnrelated.\n\n[Source](https://example.org).",
        "# One\n\n[Source](https://example.org).\n\n# Two\n\nThis is faster.",
        "This is faster.\n\n> [Source](https://example.org).",
    ] {
        assert_eq!(codes(source, "EVD001").len(), 1, "{source}");
    }
}

#[test]
fn preview_kinds_languages_and_suppression_are_respected() {
    let source = "<!-- seiso: allow STL002 -- Recorded operational observation. -->\nThe service is deployed.";
    assert!(codes(source, "STL002").is_empty());
    assert!(evaluate(source, "STL002", "howto", "", false).is_empty());
    assert!(
        evaluate(
            "The service is deployed.",
            "STL002",
            "howto",
            "languages=['zh']",
            true
        )
        .is_empty()
    );
    for kind in ["adr", "plan", "changelog", "generated"] {
        for (code, source) in [
            ("STL002", "The service is deployed."),
            ("STL004", "Version 2.3.1."),
            ("VOX002", "## Non-goals"),
            ("VOX003", "## What I changed"),
        ] {
            assert!(
                evaluate(source, code, kind, "", true).is_empty(),
                "{code}/{kind}"
            );
        }
    }
    assert_eq!(
        codes(
            "<!-- seiso: allow STL002 -- Legacy state. -->\nCheck deployment status.",
            "STL002"
        )[0]
        .code,
        "SUP002"
    );
}

#[test]
fn structural_boundaries_and_recovery_do_not_invent_a_main_flow() {
    let preamble = (0..30)
        .map(|i| format!("Background line {i}.\n"))
        .collect::<String>();
    assert!(codes(&format!("{preamble}\n1. Run it."), "ORD001").is_empty());
    assert_eq!(
        codes(&format!("{preamble}One more line.\n\n1. Run it."), "ORD001").len(),
        1
    );
    assert!(
        codes(
            &format!("{preamble}One more line.\n\n```json\n{{}}\n```"),
            "ORD001"
        )
        .is_empty()
    );
    assert!(codes("## Troubleshooting\n\n1. Restart it.", "ORD002").is_empty());
    assert!(codes("## Setup\n\n1. Run it.\n\n## FAQ\n\nDetails.", "ORD002").is_empty());
    let source =
        "## Troubleshooting\n\n### FAQ\n\n1. Restart it.\n\n## Connect\n\n```sh\napp connect\n```";
    let result = codes(source, "ORD002");
    assert_eq!(result.len(), 1);
    assert_eq!(
        &source[result[0].related[0].byte_range.start..result[0].related[0].byte_range.end],
        "```sh\napp connect\n```"
    );
    assert!(
        codes(
            "## Rationale\n\nRead the ADR.\n\n### Detail\n\nAn argument.",
            "MIX001"
        )
        .is_empty()
    );
}

#[test]
fn new_heuristics_are_preview_and_never_offer_automatic_prose_edits() {
    for code in [
        "STL002", "STL004", "RAT001", "ORD001", "ORD002", "MIX001", "VOX002", "VOX003", "EVD001",
    ] {
        let rule = seiso::rules::rule(code).unwrap();
        assert_eq!(rule.status(), "preview");
        let documentation = rule.documentation.replace("\r\n", "\n");
        let positive = documentation
            .split("```markdown\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let diagnostics = codes(positive, code);
        assert!(!diagnostics.is_empty(), "{code}");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );
    }
}

#[test]
fn new_rule_spans_and_annotations_survive_entities_unicode_and_cache_modes() {
    let source = "---\nkind: howto\nlang: en\n---\n# Guide\n\n中文: The service is dep&#108;oyed to production.\n\n## Installation\n\n1. Run `app start`.\n";
    let temporary = tempfile::tempdir().unwrap();
    let cached = seiso::cache::ParseCache::new(temporary.path().join("cache"), true);
    let cold = cached.parse(source).unwrap();
    let warm = cached.parse(source).unwrap();
    let disabled = seiso::cache::ParseCache::new(temporary.path().join("unused"), false)
        .parse(source)
        .unwrap();
    assert_eq!(classify(&cold), classify(&warm));
    assert_eq!(classify(&cold), classify(&disabled));
    let diagnostics = codes(source, "STL002");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.location.row, 7);
    assert_eq!(diagnostic.location.column, 17);
    assert_eq!(
        &source[diagnostic.byte_range.start..diagnostic.byte_range.end],
        "is dep&#108;oyed"
    );
    assert!(!temporary.path().join("unused").exists());
}
