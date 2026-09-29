use std::fs;
use std::path::Path;

use seiso::config::{CliOverrides, Config};
use seiso::diagnostics::Diagnostic;
use seiso::index::{IndexedFile, WorkspaceIndex};
use seiso::rules::cross_file::check;

const DEFINITIONS: &str = "- `host`: Server address.\n- `port`: Listening port.\n- `user`: Account name.\n- `token`: Access token.\n- `timeout`: Request timeout.\n";
const PROSE: &str = "The client reads the connection settings before opening a session. It validates the address and credentials, then applies the configured timeout to each request sent through that session.";

fn document(kind: &str, body: &str) -> String {
    format!("---\nkind: {kind}\nlang: en\n---\n\n{body}\n")
}

fn index(root: &Path, files: &[(&str, &str)], configuration: &str) -> WorkspaceIndex {
    let config = Config::parse(&format!("preview = true\n{configuration}"), root).unwrap();
    let files = files
        .iter()
        .map(|(filename, source)| {
            let path = root.join(filename);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, source).unwrap();
            let document = seiso::md::parse(source).unwrap();
            IndexedFile::new(
                (*filename).into(),
                path,
                document.into(),
                config.clone(),
                &CliOverrides::default(),
            )
            .unwrap()
        })
        .collect();
    WorkspaceIndex::new(root.to_path_buf(), files, true)
}

fn select<'a>(diagnostics: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect()
}

#[test]
fn cross_rule_documentation_examples_execute_and_snapshot_the_contract() {
    for rule in seiso::rules::rules()
        .iter()
        .filter(|rule| rule.requires_index)
    {
        let root = tempfile::tempdir().unwrap();
        let documentation = rule.documentation.replace("\r\n", "\n");
        let examples: Vec<_> = documentation
            .split("```markdown\n")
            .skip(1)
            .map(|tail| tail.split("```").next().unwrap())
            .collect();
        assert_eq!(
            examples.len(),
            2,
            "{} needs two executable examples",
            rule.code
        );
        let reference = document(
            "reference",
            &format!("# Settings\n\n{DEFINITIONS}\n{PROSE}"),
        );
        for (example_index, example) in examples.iter().enumerate() {
            let config = format!("[lint]\nselect=['{}']", rule.code);
            let workspace = index(
                root.path(),
                &[
                    ("guide.md", example),
                    ("reference.md", &reference),
                    ("docs/setup.md", &document("howto", "# Setup")),
                ],
                &config,
            );
            let report = check(&workspace);
            assert!(report.errors.is_empty());
            let diagnostics = select(&report.diagnostics, rule.code);
            assert_eq!(
                !diagnostics.is_empty(),
                example_index == 0,
                "{} example {}: {:?}",
                rule.code,
                example_index,
                report.diagnostics
            );
            if example_index == 0 {
                insta::with_settings!({
                    filters => vec![(r"/blob/v[^/]+/", "/blob/v[version]/")]
                }, {
                    insta::assert_json_snapshot!(
                        format!("{}_example", rule.code.to_ascii_lowercase()),
                        diagnostics
                    );
                });
            }
        }
    }
}

#[test]
fn definition_ownership_aggregates_all_higher_sources_and_honors_generated() {
    let root = tempfile::tempdir().unwrap();
    let lower = document("readme", DEFINITIONS);
    let higher = document("reference", DEFINITIONS);
    let generated = format!("# Generated\n\n{DEFINITIONS}");
    let config =
        "[[kinds]]\npath='generated.md'\nkind='generated'\n[lint]\nselect=['DUP001','OWN002']";
    let workspace = index(
        root.path(),
        &[
            ("guide.md", &lower),
            ("reference.md", &higher),
            ("generated.md", &generated),
        ],
        config,
    );
    let report = check(&workspace);
    let diagnostics = select(&report.diagnostics, "DUP001");
    assert_eq!(diagnostics.len(), 2);
    let guide = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.filename == "guide.md")
        .unwrap();
    assert_eq!(
        guide
            .related
            .iter()
            .map(|related| related.filename.as_str())
            .collect::<Vec<_>>(),
        ["generated.md", "reference.md"]
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.filename == "generated.md")
    );
    assert!(select(&report.diagnostics, "OWN002").is_empty());
}

#[test]
fn canonical_ties_language_and_domain_are_explicit_not_path_order() {
    let root = tempfile::tempdir().unwrap();
    let canonical = format!("---\nkind: readme\nlang: en\ncanonical: true\n---\n\n{DEFINITIONS}");
    let another = canonical.replace("readme", "plan");
    let chinese = canonical.replace("lang: en", "lang: zh");
    let config =
        "[[domains]]\npath='separate.md'\nname='independent'\n[lint]\nselect=['DUP001','OWN002']";
    let files = [
        ("a.md", canonical.as_str()),
        ("b.md", another.as_str()),
        ("translation.md", chinese.as_str()),
        ("separate.md", canonical.as_str()),
    ];
    let workspace = index(root.path(), &files, config);
    let report = check(&workspace);
    let diagnostics = select(&report.diagnostics, "OWN002");
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].filename, "a.md");
    assert_eq!(diagnostics[0].related[0].filename, "b.md");
    let mut reversed = files;
    reversed.reverse();
    assert_eq!(
        report.diagnostics,
        check(&index(root.path(), &reversed, config)).diagnostics
    );
}

fn keys(keys: &[&str]) -> String {
    keys.iter()
        .map(|key| format!("- `{key}`: Defines this setting.\n"))
        .collect()
}

#[test]
fn duplication_edges_do_not_merge_transitively() {
    let root = tempfile::tempdir().unwrap();
    let a = document("reference", &keys(&["a", "b", "c", "d", "e"]));
    let b = document("howto", &keys(&["a", "b", "c", "d", "e", "f"]));
    let c = document("readme", &keys(&["b", "c", "d", "e", "f"]));
    let workspace = index(
        root.path(),
        &[("a.md", &a), ("b.md", &b), ("c.md", &c)],
        "[lint]\nselect=['DUP001','OWN002']",
    );
    let report = check(&workspace);
    let diagnostics = select(&report.diagnostics, "DUP001");
    assert_eq!(diagnostics.len(), 2);
    let c = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.filename == "c.md")
        .unwrap();
    assert_eq!(c.related.len(), 1);
    assert_eq!(c.related[0].filename, "b.md");
}

#[test]
fn definition_heads_exclude_mentions_and_description_columns() {
    let root = tempfile::tempdir().unwrap();
    let table = "| Key | Description |\n| --- | --- |\n| `host` | `port`, `user`, `token`, `timeout` specify the connection. |\n";
    let plan = document("plan", table);
    let reference = document("reference", table);
    let workspace = index(
        root.path(),
        &[("plan.md", &plan), ("reference.md", &reference)],
        "[lint]\nselect=['DUP001','OWN001']",
    );
    let report = check(&workspace);
    assert_eq!(select(&report.diagnostics, "DUP001").len(), 1);
    assert!(select(&report.diagnostics, "OWN001").is_empty());
    for body in [
        "`host`, `port`, `user`, `token`, `timeout`.",
        "- `host`\n- `port`\n- `user`\n- `token`\n- `timeout`",
        "```\n- `host`: Address\n- `port`: Port\n- `user`: User\n- `token`: Token\n- `timeout`: Timeout\n```",
    ] {
        let lower = document("plan", body);
        let higher = document("reference", body);
        let workspace = index(
            root.path(),
            &[("plan.md", &lower), ("reference.md", &higher)],
            "[lint]\nselect=['DUP001','OWN001']",
        );
        assert!(check(&workspace).diagnostics.is_empty(), "{body}");
    }
}

#[test]
fn own001_uses_defined_table_first_column_keys() {
    let root = tempfile::tempdir().unwrap();
    let table = format!(
        "| Key | Description |\n| --- | --- |\n{}",
        ["host", "port", "user", "token", "timeout"]
            .map(|key| format!("| `{key}` | Connection setting. |\n"))
            .join("")
    );
    let plan = document("plan", &table);
    let reference = document("reference", DEFINITIONS);
    let workspace = index(
        root.path(),
        &[("plan.md", &plan), ("reference.md", &reference)],
        "[lint]\nselect=['OWN001']",
    );
    let report = check(&workspace);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].filename, "plan.md");
}

#[test]
fn restatement_uses_linked_section_only_and_allows_reversed_ownership() {
    let root = tempfile::tempdir().unwrap();
    let source = document(
        "reference",
        "# Settings\n\nSet `host`, `port`, `user`, `token`, and `timeout`.\n\nSee [details](guide.md#settings).\n",
    );
    let target = document("howto", &format!("# Settings\n\n{DEFINITIONS}"));
    let workspace = index(
        root.path(),
        &[("reference.md", &source), ("guide.md", &target)],
        "[lint]\nselect=['DUP002']",
    );
    let report = check(&workspace);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].filename, "guide.md");
    assert_eq!(report.diagnostics[0].related[0].filename, "reference.md");
    let target = document(
        "howto",
        &format!("# Settings\n\nNo definitions here.\n\n# Other\n\n{DEFINITIONS}"),
    );
    assert!(
        check(&index(
            root.path(),
            &[("reference.md", &source), ("guide.md", &target)],
            "[lint]\nselect=['DUP002']"
        ))
        .diagnostics
        .is_empty()
    );
    let source = format!("{source}\nMore text after the pointer.\n");
    assert!(
        check(&index(
            root.path(),
            &[("reference.md", &source), ("guide.md", &target)],
            "[lint]\nselect=['DUP002']"
        ))
        .diagnostics
        .is_empty()
    );
}

#[test]
fn paragraph_candidates_ignore_short_text_quotes_and_different_domains() {
    let root = tempfile::tempdir().unwrap();
    for body in [
        "Short shared sentence.",
        &format!("> {PROSE}"),
        &format!("- {PROSE}"),
        &format!("```text\n{PROSE}\n```"),
    ] {
        let lower = document("howto", body);
        let higher = document("reference", body);
        assert!(
            check(&index(
                root.path(),
                &[("guide.md", &lower), ("reference.md", &higher)],
                "[lint]\nselect=['DUP003']"
            ))
            .diagnostics
            .is_empty()
        );
    }
    let lower = document("howto", PROSE);
    let higher = document("reference", &PROSE.replace("each request", "every request"));
    let workspace = index(
        root.path(),
        &[("guide.md", &lower), ("reference.md", &higher)],
        "[lint]\nselect=['DUP003']",
    );
    assert_eq!(check(&workspace).diagnostics.len(), 1);
    let config = "[[domains]]\npath='reference.md'\nname='other'\n[lint]\nselect=['DUP003']";
    assert!(
        check(&index(
            root.path(),
            &[("guide.md", &lower), ("reference.md", &higher)],
            config
        ))
        .diagnostics
        .is_empty()
    );
}

#[test]
fn anchors_preserve_target_related_locations_and_unknown_targets_stay_incomplete() {
    let root = tempfile::tempdir().unwrap();
    let source = document(
        "howto",
        "[missing](target.md#removed)\n\n[repeat](target.md#settings-1)\n\n[custom](target.md#custom)\n\n[excluded](excluded.md#unknown)\n\n[absent](absent.md#unknown)\n",
    );
    fs::write(root.path().join("excluded.md"), "# Hidden").unwrap();
    let target = "# Settings\n\n# Settings\n\n<a id=\"custom\"></a>\n";
    let workspace = index(
        root.path(),
        &[("guide.md", &source), ("target.md", target)],
        "[[kinds]]\npath='target.md'\nkind='generated'\n[lint]\nselect=['LNK002']",
    );
    let report = check(&workspace);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].related[0].filename, "target.md");
    assert!(report.incomplete["guide.md"].contains("LNK002"));
}

#[test]
fn catalog_directory_allowance_is_exact_and_configuration_relative() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("docs/nested")).unwrap();
    let source = document("howto", "[index](docs/)\n\n[nested](docs/nested/)");
    let config = "[lint]\nselect=['PTR002']\n[lint.ptr]\ncatalog-dirs=['docs/']";
    let report = check(&index(root.path(), &[("guide.md", &source)], config));
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].related[0].filename, "docs/nested");
}

#[test]
fn incomplete_inventory_marks_every_enabled_duplication_rule_without_hiding_known_hits() {
    let root = tempfile::tempdir().unwrap();
    let lower = document("plan", DEFINITIONS);
    let higher = document("reference", DEFINITIONS);
    let mut workspace = index(
        root.path(),
        &[("plan.md", &lower), ("reference.md", &higher)],
        "[lint]\nselect=['DUP','OWN']",
    );
    workspace.complete = false;
    let report = check(&workspace);
    assert_eq!(report.incomplete["plan.md"].len(), 5);
    assert_eq!(select(&report.diagnostics, "DUP001").len(), 1);
    assert_eq!(select(&report.diagnostics, "OWN001").len(), 1);
}

#[test]
fn malformed_frontmatter_cannot_claim_canonical_ownership() {
    let root = tempfile::tempdir().unwrap();
    let invalid = format!("---\nkind: 123\ncanonical: true\n---\n\n{DEFINITIONS}");
    let reference = document("reference", DEFINITIONS);
    let report = check(&index(
        root.path(),
        &[("invalid.md", &invalid), ("reference.md", &reference)],
        "[lint]\nselect=['DUP001','OWN002']",
    ));
    assert!(report.diagnostics.is_empty());
}

#[test]
fn bare_lists_of_multiple_inline_code_mentions_are_not_definitions() {
    let root = tempfile::tempdir().unwrap();
    let mentions = "- `host`, `port`, `user`, `token`, and `timeout`.";
    let lower = document("howto", mentions);
    let higher = document("reference", mentions);
    let report = check(&index(
        root.path(),
        &[("guide.md", &lower), ("reference.md", &higher)],
        "[lint]\nselect=['DUP001']",
    ));
    assert!(report.diagnostics.is_empty());
}
