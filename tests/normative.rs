use std::collections::BTreeSet;

use seiso::config::Config;
use seiso::diagnostics::Diagnostic;

fn check(code: &str, markdown: &str, settings: &str) -> Vec<Diagnostic> {
    let root = std::env::current_dir().unwrap();
    let config = Config::parse(settings, &root).unwrap();
    let document = seiso::md::parse(markdown).unwrap();
    seiso::rules::normative::check(
        &document,
        "docs/example.md",
        &root.join("docs/example.md"),
        &root,
        &config,
        &BTreeSet::from([code.to_string()]),
    )
}

fn language(lang: &str, text: &str) -> String {
    format!("---\nkind: howto\nlang: {lang}\n---\n{text}\n")
}

fn assert_one(code: &str, source: &str, expected: &str) {
    let diagnostics = check(code, source, "");
    assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, code);
    assert!(!diagnostic.suggestion.is_empty());
    assert!(diagnostic.fix.is_none());
    assert_eq!(
        &source[diagnostic.byte_range.start..diagnostic.byte_range.end],
        expected
    );
}

#[test]
fn current_state_values_include_inline_code_and_cjk_counts() {
    for (lang, text, expected) in [
        ("en", "The service currently uses `v1.2.3`.", "v1.2.3"),
        ("en", "The latest build contains 12 migrations.", "12"),
        ("en", "We currently run `nginx:stable`.", "nginx:stable"),
        ("en", "Currently deployed: a1b2c3d.", "a1b2c3d"),
        ("zh", "目前有12个迁移。", "12"),
        ("zh", "目前使用 `v1.2.3`。", "v1.2.3"),
        ("ja", "現在のバージョンは `v1.2.3` です。", "v1.2.3"),
    ] {
        assert_one("STL001", &language(lang, text), expected);
    }
}

#[test]
fn current_state_values_exclude_constraints_urls_and_separate_sentences() {
    for (lang, text) in [
        ("en", "Currently requires `1.2.3` or later."),
        ("en", "Currently supports at least 2 workers."),
        ("en", "Currently requires `1.2.3+`."),
        ("en", "Currently supports ≥ 2 workers."),
        ("en", "The current URL is https://example.com/latest/1.2.3."),
        (
            "en",
            "Currently see [the release](https://example.com/1.2.3).",
        ),
        ("en", "`currently` is a keyword. The version is 1.2.3."),
        ("en", "Concurrently, 12 workers run."),
        ("en", "Currently the mode is node12."),
        ("en", "Currently stable. Version 1.2.3 is the example."),
        ("zh", "目前要求版本 1.2.3 以上。"),
        ("ja", "現在はバージョン 1.2.3 以上が必要です。"),
    ] {
        assert!(
            check("STL001", &language(lang, text), "").is_empty(),
            "{text}"
        );
    }
}

#[test]
fn commit_contexts_require_adjacent_bounded_identifiers() {
    for (lang, text, expected) in [
        ("en", "Pinned commit `a1b2c3d`.", "a1b2c3d"),
        ("en", "SHA: A1B2C3D4", "A1B2C3D4"),
        (
            "en",
            "Build = 0123456789012345678901234567890123456789",
            "0123456789012345678901234567890123456789",
        ),
        ("zh", "部署提交abcdef0。", "abcdef0"),
        ("ja", "コミット `abcdef0` を使用します。", "abcdef0"),
    ] {
        assert_one("STL003", &language(lang, text), expected);
    }
    for text in [
        "commit abcdef",
        "commit 01234567890123456789012345678901234567890",
        "commit abcdef0suffix",
        "recommit abcdef0",
        "commitment abcdef0",
        "commit named abcdef0",
        "[commit](https://example.com/abcdef0)",
        "commit https://example.com/abcdef0",
        "```text\ncommit abcdef0\n```",
    ] {
        assert!(check("STL003", text, "").is_empty(), "{text}");
    }
}

#[test]
fn root_pointers_resolve_relative_reference_and_forge_links() {
    for (source, expected) in [
        ("See [configuration](/).", "/"),
        ("See [configuration](../).", "../"),
        ("See [configuration](%2e%2e/).", "%2e%2e/"),
        ("See [configuration](../docs/..).", "../docs/.."),
        (
            "See [configuration][config].\n\n[config]: /",
            "[configuration][config]",
        ),
        (
            "See [configuration](https://github.com/example/project).",
            "https://github.com/example/project",
        ),
        (
            "See [configuration](https://gitlab.com/example/project/).",
            "https://gitlab.com/example/project/",
        ),
    ] {
        assert_one("PTR001", source, expected);
    }
    assert_one("PTR001", &language("zh", "定义在[仓库](/)。"), "/");
    assert_one(
        "PTR001",
        &language("ja", "[リポジトリ](/)を参照してください。"),
        "/",
    );
}

#[test]
fn root_pointers_require_a_pointer_and_an_unqualified_repository_root() {
    for text in [
        "The [repository](/) hosts the project.",
        "See [configuration](./).",
        "See [configuration](../src/config.rs).",
        "See [configuration](https://github.com/example/project/blob/main/config.rs).",
        "See [configuration](https://github.com/example/project#configuration).",
        "See [configuration](https://example.com/example/project).",
        "See [Actions](https://github.com/features/actions).",
        "See [organization](https://github.com/orgs/example).",
        "See [sign in](https://gitlab.com/users/sign_in).",
        "See [sign in](https://codeberg.org/user/login).",
        "See [account](https://bitbucket.org/account/settings).",
        "See [configuration](../../).",
        "See ![diagram](/).",
        "`See` [configuration](/).",
        "Oversee [configuration](/).",
        "See [configuration](//github.com/example/project).",
        "See [configuration](${ROOT}).",
    ] {
        assert!(check("PTR001", text, "").is_empty(), "{text}");
    }
}

#[test]
fn reference_root_pointer_uses_its_paragraph_for_block_suppression() {
    let source = "---\nkind: reference\n---\n<!-- seiso: allow PTR001 -- This catalog introduces the whole project. -->\nSee [the project][project].\n\n[project]: /\n";
    let root = std::env::current_dir().unwrap();
    let path = root.join("docs/example.md");
    let config =
        Config::parse("preview = true\n[lint]\nselect = ['PTR001', 'SUP']", &root).unwrap();
    let document = seiso::md::parse(source).unwrap();
    let result = seiso::rules::check(&seiso::rules::CheckContext {
        document: &document,
        filename: "docs/example.md",
        path: &path,
        workspace_root: &root,
        config: &config,
        overrides: &Default::default(),
    })
    .unwrap();
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.suppressions[0].states["PTR001"],
        seiso::rules::suppression::SuppressionState::Active { count: 1 }
    );
}

#[test]
fn root_link_with_multiple_label_sentences_reports_once() {
    assert_one("PTR001", "[See this. See more.](/)", "/");
}

#[test]
fn source_pointer_accepts_targets_in_all_declared_fragment_kinds() {
    for text in [
        "See the source in src/config.rs.",
        "See the source in `src/config.rs`.",
        "See the source in [config.rs](https://example.com).",
        "See the [source](https://github.com/example/project/blob/main/src/config.rs).",
        "See the source in `Config::load`.",
        "See the source in `parse_config`.",
        "See the source in `Config`.",
        "See the source in `parse`.",
        "See the source in `CONFIG_VERSION`.",
        "See the source in `parse()`.",
        "See the source in `Dockerfile`.",
        "See the [source](#configuration).",
    ] {
        assert!(check("PTR003", text, "").is_empty(), "{text}");
    }
    assert_one("PTR003", "See the source.", "See the source");
    assert_one(
        "PTR003",
        "See the [source](https://github.com/example/project).",
        "See the [source",
    );
    assert_one("PTR003", &language("zh", "具体行为见源码。"), "见源码");
    assert_one(
        "PTR003",
        &language("ja", "詳細はソースを参照してください。"),
        "ソースを参照",
    );
    assert_one("PTR003", "See the source in `123`.", "See the source");
}

#[test]
fn rationale_headings_match_design_choices_without_reporting_troubleshooting() {
    for (lang, source, expected) in [
        ("en", "## Why we chose Redis", "Why we chose"),
        ("zh", "## 为什么选择 Redis", "为什么选择"),
        ("zh", "## 为什么不用 Redis", "为什么不用"),
        ("ja", "## なぜRedisを選んだのか", "なぜRedisを選んだ"),
    ] {
        assert_one("RAT002", &language(lang, source), expected);
    }
    for (lang, source) in [
        ("en", "Why we chose Redis is recorded in the ADR."),
        ("en", "## Why does the connection fail?"),
        ("en", "## Why we chosefully document failures"),
        ("zh", "## 为什么连接失败"),
        ("ja", "## なぜ接続に失敗するのか"),
        ("en", "## `Why we chose Redis`"),
    ] {
        assert!(
            check("RAT002", &language(lang, source), "").is_empty(),
            "{source}"
        );
    }
}

#[test]
fn conversation_markers_handle_case_entities_and_formatting_without_code_splicing() {
    for (lang, text, expected) in [
        (
            "en",
            "AS YOU REQUESTED, the guide follows.",
            "AS YOU REQUESTED",
        ),
        ("en", "Here's the updated guide.", "Here's the updated"),
        (
            "en",
            "Here&#39;s the updated guide.",
            "Here&#39;s the updated",
        ),
        (
            "en",
            "As **you** requested, install the tool.",
            "As **you** requested",
        ),
        ("zh", "根据你的要求，文档如下。", "根据你的要求"),
        (
            "ja",
            "ご要望に応じて、手順を更新しました。",
            "ご要望に応じて",
        ),
    ] {
        assert_one("VOX001", &language(lang, text), expected);
    }
    for text in [
        "`as you requested` is an example.",
        "As `you` requested, install the tool.",
        "As you requestedly document this.",
        "The URL is https://example.com/as%20you%20requested.",
        "```text\nAs you requested.\n```",
        "As you [requested](https://example.com)ly documented.",
    ] {
        assert!(check("VOX001", text, "").is_empty(), "{text}");
    }
}

#[test]
fn reports_about_the_requester_are_remnants_but_end_user_behavior_is_not() {
    for (lang, text, expected) in [
        ("zh", "经用户确认，发布流程改用手动审批。", "经用户确认"),
        ("zh", "用户已授权删除旧的配置目录。", "用户已授权"),
        ("zh", "已与用户确认，保留旧接口。", "已与用户确认"),
        (
            "zh",
            "根据用户的要求，默认端口改为 8080。",
            "根据用户的要求",
        ),
        (
            "en",
            "The user confirmed that the old flag can go.",
            "The user confirmed",
        ),
        (
            "en",
            "As the user requested, the guide skips Docker.",
            "As the user requested",
        ),
        (
            "en",
            "After the review, the user confirmed that the old flag can go.",
            "the user confirmed",
        ),
        (
            "zh",
            "为了兼容旧客户端，经用户确认，保留旧接口。",
            "经用户确认",
        ),
    ] {
        assert_one("VOX001", &language(lang, text), expected);
    }
    for (lang, text) in [
        ("zh", "未经用户授权，应用不得读取通讯录。"),
        ("zh", "删除操作须经用户确认。"),
        ("zh", "订单经用户确认后发货。"),
        ("zh", "列表显示用户已授权的应用。"),
        ("zh", "如果用户已同意隐私政策，则跳过弹窗。"),
        ("zh", "用户授权后，应用获得访问令牌。"),
        (
            "en",
            "Once the user approved the request, the app receives a token.",
        ),
        (
            "en",
            "If the user has authorized the app, skip the consent screen.",
        ),
        (
            "en",
            "Verify that the user has authorized the app before requesting a token.",
        ),
        (
            "en",
            "Ensure that the user confirmed the deletion before removing the account.",
        ),
        ("en", "Make sure the user approved the request."),
        ("zh", "请确认用户已授权该应用。"),
        ("zh", "检查请求前必须确保用户已同意条款。"),
    ] {
        assert!(
            check("VOX001", &language(lang, text), "").is_empty(),
            "{text}"
        );
    }
}

#[test]
fn extensions_are_literal_language_specific_and_language_selection_is_respected() {
    let settings = r#"
[lint.lexicon.en]
extend-stale-markers = ["as of today"]
extend-constraint-markers = ["supported floor"]
extend-commit-contexts = ["revision"]
extend-pointer-markers = ["consult"]
extend-source-pointers = ["inspect the implementation"]
extend-rationale-headings = ["reasons for choosing"]
extend-conversation-markers = ["per your request"]
"#;
    for (code, source) in [
        ("STL001", "As of today, the service has 2 workers."),
        ("STL003", "Revision abcdef0."),
        ("PTR001", "Consult [the repository](/)."),
        ("PTR003", "Inspect the implementation."),
        ("RAT002", "## Reasons for choosing Redis"),
        ("VOX001", "Per your request, here are the steps."),
    ] {
        assert_eq!(
            check(code, &language("en", source), settings).len(),
            1,
            "{code}"
        );
    }
    assert!(check("STL001", "As of today, the supported floor is 2.", settings).is_empty());
    assert!(check("VOX001", &language("zh", "per your request"), settings).is_empty());
    assert!(
        check(
            "VOX001",
            &language("en", "As you requested."),
            "[lint]\nlanguages = ['zh']"
        )
        .is_empty()
    );
}

#[test]
fn rules_obey_enabled_set_and_preserve_unicode_diagnostic_locations() {
    let root = std::env::current_dir().unwrap();
    let document = seiso::md::parse("---\nlang: zh\n---\n你好，根据你的要求，开始。\n").unwrap();
    let config = Config::defaults(&root).unwrap();
    assert!(
        seiso::rules::normative::check(
            &document,
            "a.md",
            &root.join("a.md"),
            &root,
            &config,
            &BTreeSet::new()
        )
        .is_empty()
    );
    let result = check("VOX001", &document.source, "");
    assert_eq!(result[0].location.row, 4);
    assert_eq!(result[0].location.column, 4);
}

#[test]
fn multibyte_urls_preserve_the_positions_of_later_prose() {
    let source = language(
        "zh",
        "https://example.com/根据你的要求 。 根据你的要求，开始。",
    );
    assert_one("VOX001", &source, "根据你的要求");
    let diagnostic = &check("VOX001", &source, "")[0];
    assert_eq!(
        diagnostic.byte_range.start,
        source.rfind("根据你的要求").unwrap()
    );
}
