mod common;
use common::{CheckContext, check};
use std::fs;
use std::path::Path;

use seiso::config::{CliOverrides, Config};
use seiso::rules::CheckResult;

fn evaluate(root: &Path, source: &str, configuration: &str, preview: bool) -> CheckResult {
    let document = seiso::md::parse(source).unwrap();
    let config = Config::parse(configuration, root).unwrap();
    check(&CheckContext {
        document: &document,
        filename: "docs/guide.md",
        path: &root.join("docs/guide.md"),
        workspace_root: root,
        config: &config,
        overrides: &CliOverrides {
            preview,
            ..CliOverrides::default()
        },
    })
    .unwrap()
}

#[test]
fn accepted_rules_run_by_default_and_future_rules_do_not_claim_execution() {
    let root = tempfile::tempdir().unwrap();
    let result = evaluate(root.path(), "No declaration.\n", "", false);
    assert_eq!(
        result.enabled_rules,
        ["KND001", "KND002", "LNK001", "SUP001", "SUP002"]
    );
    assert_eq!(result.diagnostics[0].code, "KND001");
    let result = evaluate(root.path(), "No declaration.\n", "", true);
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        ["KND001"]
    );
    assert_eq!(
        result.enabled_rules,
        ["KND001", "KND002", "LNK001", "SUP001", "SUP002"]
    );
}

#[test]
fn kind_errors_do_not_fall_back_to_a_configured_exemption() {
    let root = tempfile::tempdir().unwrap();
    let mapping = "[[kinds]]\npath = '**/*.md'\nkind = 'generated'\n";
    for (source, code) in [
        ("---\nkind: guide\n---\n# Guide\n", "KND002"),
        ("---\nkind: generated\n---\n# Guide\n", "KND002"),
        ("---\nkind: [\n---\n# Guide\n", "KND001"),
    ] {
        let result = evaluate(root.path(), source, mapping, true);
        assert_eq!(result.kind.value, None);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, code);
        assert_eq!(result.diagnostics[0].location.row, 1);
    }
    let result = evaluate(
        root.path(),
        "<!-- seiso: allow BAD -->\n[link](missing.md)",
        mapping,
        true,
    );
    assert_eq!(
        result.kind.value.map(seiso::rules::Kind::as_str),
        Some("generated")
    );
    assert!(result.diagnostics.is_empty());
    assert!(result.enabled_rules.is_empty());
    assert!(result.suppressions.is_empty());
}

#[test]
fn frontmatter_overrides_mapping_and_last_path_mapping_wins() {
    let root = tempfile::tempdir().unwrap();
    let config = "[[kinds]]\npath='**/*.md'\nkind='generated'\n[[kinds]]\npath='docs/**'\nkind='reference'\n";
    let result = evaluate(root.path(), "# Guide", config, true);
    assert_eq!(
        result.kind.value.map(seiso::rules::Kind::as_str),
        Some("reference")
    );
    let result = evaluate(
        root.path(),
        "---\nkind: changelog\n---\n# Release",
        config,
        true,
    );
    assert_eq!(
        result.kind.value.map(seiso::rules::Kind::as_str),
        Some("changelog")
    );
    assert!(!result.enabled_rules.contains(&"STL001".into()));
}

#[test]
fn local_links_use_current_filesystem_and_document_directory() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    fs::write(root.path().join("assets/配置 file.txt"), "data").unwrap();
    fs::write(
        root.path().join("docs/target.md"),
        "---\nkind: generated\n---",
    )
    .unwrap();
    let source = "---\nkind: howto\n---\n[existing](target.md#not-checked)\n\n![image](../assets/%E9%85%8D%E7%BD%AE%20file.txt?raw=1)\n\n[directory](../assets/)\n\n[missing][ref]\n\n[ref]: old.md\n";
    let config = "[lint]\nselect=['LNK001']\n";
    let first = evaluate(root.path(), source, config, true);
    assert!(first.errors.is_empty());
    assert_eq!(first.diagnostics.len(), 1);
    assert_eq!(first.diagnostics[0].location.row, 10);
    assert_eq!(
        &source[first.diagnostics[0].byte_range.start..first.diagnostics[0].byte_range.end],
        "[missing][ref]"
    );
    fs::write(root.path().join("docs/old.md"), "Restored").unwrap();
    assert!(
        evaluate(root.path(), source, config, true)
            .diagnostics
            .is_empty()
    );
    fs::remove_file(root.path().join("docs/target.md")).unwrap();
    assert_eq!(
        evaluate(root.path(), source, config, true)
            .diagnostics
            .len(),
        1
    );
}

#[test]
fn links_do_not_guess_external_paths_or_malformed_urls() {
    let root = tempfile::tempdir().unwrap();
    let source = "[external](https://example.invalid/not-there)\n\n[anchor](#missing)\n\n[email](mailto:docs@example.invalid)\n\n[outside](../../elsewhere.md)\n\n<!-- seiso: allow LNK001 -- This URL is pending correction. -->\n[invalid](broken%zz.md)\n";
    let result = evaluate(
        root.path(),
        source,
        "[lint]\nselect=['LNK001','SUP002']",
        true,
    );
    assert!(result.diagnostics.is_empty());
    assert!(result.errors.is_empty());
    assert_eq!(result.suppressions.len(), 1);
}

#[test]
fn root_relative_paths_templates_and_stdin_self_links_preserve_three_way_resolution() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("README.md"), "# Root").unwrap();
    let config = "[lint]\nselect=['LNK001','SUP002']";
    let source = "[root](/README.md)\n\n[self](guide.md)\n\n[missing](/missing.md)\n";
    let result = evaluate(root.path(), source, config, true);
    assert!(result.errors.is_empty());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, "LNK001");
    assert!(result.diagnostics[0].message.contains("/missing.md"));
    for destination in [
        "../../elsewhere.md",
        "${DOCS}/guide.md",
        "{{base}}/guide.md",
        "%7Broute%7D/index.md",
    ] {
        let source = format!(
            "<!-- seiso: allow LNK001 -- The build chooses the destination. -->\n[unknown]({destination})\n"
        );
        let result = evaluate(root.path(), &source, config, true);
        assert!(
            result.diagnostics.is_empty(),
            "{destination}: {:?}",
            result.diagnostics
        );
        assert!(matches!(
            result.suppressions[0].states["LNK001"],
            seiso::rules::suppression::SuppressionState::Incomplete { count: 0 }
        ));
    }
}

#[cfg(unix)]
#[test]
fn inaccessible_links_leave_suppressions_undetermined() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    std::os::unix::fs::symlink("loop", root.path().join("docs/loop")).unwrap();
    let result = evaluate(
        root.path(),
        "<!-- seiso: allow LNK001 -- Loop target needs inspection. -->\n[loop](loop/file.md)\n",
        "[lint]\nselect=['LNK001','SUP002']",
        true,
    );
    assert!(result.diagnostics.is_empty());
    assert!(!result.errors.is_empty());
}

#[cfg(unix)]
#[test]
fn missing_child_of_external_symlink_is_outside_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("docs/external")).unwrap();
    let result = evaluate(
        root.path(),
        "[outside](external/missing.md)\n",
        "[lint]\nselect=['LNK001']",
        true,
    );
    assert!(result.diagnostics.is_empty());
    assert!(result.errors.is_empty());
}

#[test]
fn rule_documentation_examples_execute_the_published_contract() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("docs")).unwrap();
    for rule in seiso::rules::rules() {
        if rule.requires_index {
            continue;
        }
        let documentation = rule.documentation.replace("\r\n", "\n");
        let examples: Vec<_> = documentation
            .split("```markdown\n")
            .skip(1)
            .map(|tail| tail.split("```").next().unwrap())
            .collect();
        assert_eq!(
            examples.len(),
            2,
            "{} needs a positive and negative Markdown example",
            rule.code
        );
        let selected = if rule.code == "SUP002" {
            "'SUP002','LNK001'".to_owned()
        } else {
            format!("'{}'", rule.code)
        };
        let config =
            format!("[[kinds]]\npath='docs/**'\nkind='howto'\n[lint]\nselect=[{selected}]\n");
        for (index, example) in examples.into_iter().enumerate() {
            let result = evaluate(root.path(), example, &config, true);
            let found = result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == rule.code);
            assert_eq!(
                found,
                index == 0,
                "{} example {}: {:?}",
                rule.code,
                index + 1,
                result.diagnostics
            );
            if index == 0 {
                insta::with_settings!({
                    filters => vec![(r"/blob/v[^/]+/", "/blob/v[version]/")]
                }, {
                    insta::assert_json_snapshot!(
                        format!("{}_example", rule.code.to_ascii_lowercase()),
                        result.diagnostics
                    );
                });
            }
        }
    }
}
