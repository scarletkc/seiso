mod common;
use common::write;

use std::fs;
use std::path::Path;

use seiso::config::{CliOverrides, Config, ConfigError, Workspace};
use seiso::rules::{Kind, rule_codes};
use tempfile::{TempDir, tempdir};

fn parse(source: &str) -> (TempDir, Config) {
    let dir = tempdir().unwrap();
    let config = Config::parse(source, dir.path()).unwrap();
    (dir, config)
}

fn rules(config: &Config, kind: Option<&str>, overrides: &CliOverrides) -> Vec<&'static str> {
    config
        .enabled_rules(
            Path::new("docs/page.md"),
            kind.and_then(Kind::from_name),
            overrides,
        )
        .unwrap()
}

#[test]
fn default_patterns_include_root_and_nested_markdown() {
    let (_dir, config) = parse("");
    assert!(config.includes(Path::new("README.md")));
    assert!(config.includes(Path::new("docs/中文/ガイド.md")));
    assert!(config.includes(Path::new("docs/notes.markdown")));
    assert!(!config.includes(Path::new("README.txt")));
    assert!(!config.excludes(Path::new("docs/page.md")));
}

#[test]
fn paths_are_relative_to_config_and_normalized() {
    let (dir, config) = parse("include = ['docs/*.md']\nexclude = ['docs/old.md']");
    assert!(config.includes(&dir.path().join("docs/sub/../page.md")));
    assert!(config.includes(Path::new("docs\\page.md")));
    assert!(!config.includes(Path::new("docs/sub/page.md")));
    assert!(config.excludes(Path::new("docs/old.md")));
    assert!(
        config
            .relative_path(&dir.path().join("../outside.md"))
            .is_none()
    );
}

#[test]
fn last_matching_kind_and_domain_win() {
    let (_dir, config) = parse(
        r#"
[[kinds]]
path = "docs/**"
kind = "howto"
[[kinds]]
path = "docs/reference/**"
kind = "reference"
[[kinds]]
path = "docs/reference/generated/**"
kind = "generated"
[[domains]]
path = "docs/**"
name = "all-docs"
[[domains]]
path = "docs/reference/**"
name = "api"
"#,
    );
    assert_eq!(
        config.kind_for(Path::new("docs/intro.md")),
        Some(Kind::Howto)
    );
    assert_eq!(
        config.kind_for(Path::new("docs/reference/config.md")),
        Some(Kind::Reference)
    );
    assert_eq!(
        config.kind_for(Path::new("docs/reference/generated/client.md")),
        Some(Kind::Generated)
    );
    assert_eq!(
        config.domain_for(Path::new("docs/reference/config.md")),
        Some("api")
    );
    assert_eq!(config.domain_for(Path::new("README.md")), None);
}

#[test]
fn same_directory_config_precedence() {
    let dir = tempdir().unwrap();
    write(dir.path(), ".seiso.toml", "preview = true");
    write(dir.path(), "seiso.toml", "preview = false");
    write(dir.path(), "pyproject.toml", "[tool.seiso]\ninclude = []");
    let workspace = Workspace::discover(dir.path(), None).unwrap();
    assert_eq!(
        workspace.config.source.unwrap(),
        dir.path().join(".seiso.toml")
    );
    fs::remove_file(dir.path().join(".seiso.toml")).unwrap();
    let workspace = Workspace::discover(dir.path(), None).unwrap();
    assert_eq!(
        workspace.config.source.unwrap(),
        dir.path().join("seiso.toml")
    );
    fs::remove_file(dir.path().join("seiso.toml")).unwrap();
    let workspace = Workspace::discover(dir.path(), None).unwrap();
    assert!(workspace.config.settings.include.is_empty());
    assert_eq!(
        workspace.config.source.unwrap(),
        dir.path().join("pyproject.toml")
    );
}

#[test]
fn pyproject_without_seiso_does_not_stop_discovery() {
    let dir = tempdir().unwrap();
    write(dir.path(), "seiso.toml", "preview = true");
    write(
        dir.path(),
        "docs/pyproject.toml",
        "[tool.unrelated]\nname = 'test'",
    );
    let workspace = Workspace::discover(&dir.path().join("docs"), None).unwrap();
    assert_eq!(workspace.root, dir.path());
    assert!(workspace.config.settings.preview);
    assert_eq!(
        workspace
            .config_for(Path::new("docs/file.md"))
            .unwrap()
            .source,
        workspace.config.source
    );
}

#[test]
fn nested_config_replaces_parent_without_implicit_inheritance() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "seiso.toml",
        "preview = true\nexclude = ['docs/**']\n[[kinds]]\npath = '**/*.md'\nkind = 'howto'",
    );
    write(
        dir.path(),
        "docs/.seiso.toml",
        "[[kinds]]\npath = 'reference/**'\nkind = 'reference'",
    );
    let workspace = Workspace::discover(dir.path(), None).unwrap();
    let nested = workspace
        .config_for(Path::new("docs/reference/page.md"))
        .unwrap();
    assert!(!nested.settings.preview);
    assert!(!nested.excludes(&dir.path().join("docs/reference/page.md")));
    assert_eq!(
        nested.kind_for(&dir.path().join("docs/reference/page.md")),
        Some(Kind::Reference)
    );
    assert_eq!(nested.kind_for(&dir.path().join("docs/guide.md")), None);
}

#[test]
fn nearest_config_sets_workspace_root() {
    let dir = tempdir().unwrap();
    write(dir.path(), "seiso.toml", "preview = true");
    write(dir.path(), "docs/seiso.toml", "preview = false");
    fs::create_dir_all(dir.path().join("docs/guides")).unwrap();
    let workspace = Workspace::discover(&dir.path().join("docs/guides"), None).unwrap();
    assert_eq!(workspace.root, dir.path().join("docs"));
}

#[test]
fn git_marker_and_cwd_are_fallback_roots() {
    let dir = tempdir().unwrap();
    write(dir.path(), ".git", "gitdir: somewhere");
    fs::create_dir_all(dir.path().join("docs/subdir")).unwrap();
    let workspace = Workspace::discover(&dir.path().join("docs/subdir"), None).unwrap();
    assert_eq!(workspace.root, dir.path());
    fs::remove_file(dir.path().join(".git")).unwrap();
    let workspace = Workspace::discover(&dir.path().join("docs/subdir"), None).unwrap();
    assert_eq!(workspace.root, dir.path().join("docs/subdir"));
}

#[test]
fn explicit_config_pins_policy() {
    let dir = tempdir().unwrap();
    write(dir.path(), "custom.toml", "preview = true");
    write(dir.path(), "docs/seiso.toml", "preview = false");
    let workspace = Workspace::discover(dir.path(), Some(Path::new("custom.toml"))).unwrap();
    assert_eq!(workspace.root, dir.path());
    assert!(
        workspace
            .config_for(Path::new("docs/page.md"))
            .unwrap()
            .settings
            .preview
    );
}

#[test]
fn config_for_rejects_outside_paths_after_normalization() {
    let dir = tempdir().unwrap();
    write(dir.path(), "seiso.toml", "");
    let workspace = Workspace::discover(dir.path(), None).unwrap();
    assert!(matches!(
        workspace.config_for(Path::new("../outside.md")),
        Err(ConfigError::OutsideWorkspace { .. })
    ));
    assert!(workspace.config_for(Path::new("docs/../page.md")).is_ok());
}

#[test]
fn explicit_extend_merges_tables_and_replaces_arrays() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "base.toml",
        r#"
include = ["legacy/*.md"]
preview = true
[[kinds]]
path = "legacy/**"
kind = "adr"
[lint]
select = ["DUP", "KND"]
ignore = ["KND002"]
[lint.dup]
min-identifiers = 7
min-jaccard = 0.7
[lint.per-file-ignores]
"old/**" = ["DUP"]
"override/**" = ["ALL"]
"#,
    );
    write(
        dir.path(),
        "docs/seiso.toml",
        r#"
extend = "../base.toml"
include = ["**/*.md"]
[[kinds]]
path = "reference/**"
kind = "reference"
[lint]
select = ["ALL"]
[lint.dup]
min-jaccard = 0.9
[lint.per-file-ignores]
"override/**" = ["KND"]
"#,
    );
    let config = Config::load(&dir.path().join("docs/seiso.toml")).unwrap();
    assert!(config.settings.preview);
    assert_eq!(config.settings.lint.select, ["ALL"]);
    assert_eq!(config.settings.lint.ignore, ["KND002"]);
    assert_eq!(config.settings.lint.dup.min_identifiers, 7);
    assert_eq!(config.settings.lint.dup.min_jaccard, 0.9);
    assert_eq!(
        config.settings.lint.per_file_ignores["override/**"],
        ["KND"]
    );
    assert_eq!(config.settings.lint.per_file_ignores["old/**"], ["DUP"]);
    assert_eq!(config.settings.kinds.len(), 1);
    assert_eq!(
        config.kind_for(&dir.path().join("docs/reference/page.md")),
        Some(Kind::Reference)
    );
}

#[test]
fn extension_lists_accumulate_across_three_levels_and_apply_cli_overrides() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "base.toml",
        r#"
exclude = ["vendor/**"]
extend-exclude = ["generated/**"]
[lint]
select = ["KND"]
extend-select = ["DUP"]
ignore = ["KND002"]
extend-ignore = ["STL"]
"#,
    );
    write(
        dir.path(),
        "middle.toml",
        r#"
extend = "base.toml"
exclude = ["legacy/**"]
extend-exclude = ["build/**"]
[lint]
select = ["LNK"]
extend-select = ["PTR"]
extend-ignore = ["LNK001"]
"#,
    );
    write(
        dir.path(),
        "child/seiso.toml",
        r#"
extend = "../middle.toml"
extend-exclude = ["nested/**"]
[lint]
extend-select = ["KND001"]
"#,
    );

    let config = Config::load(&dir.path().join("child/seiso.toml")).unwrap();
    assert_eq!(
        config.settings.exclude,
        ["legacy/**", "generated/**", "build/**", "nested/**"]
    );
    assert_eq!(config.settings.lint.select, ["LNK", "DUP", "PTR", "KND001"]);
    assert_eq!(config.settings.lint.ignore, ["KND002", "STL", "LNK001"]);
    assert!(config.excludes(Path::new("nested/page.md")));
    assert!(config.excludes(Path::new("generated/page.md")));
    assert!(!config.excludes(Path::new("vendor/page.md")));

    let reported = serde_json::to_value(&config.settings).unwrap();
    assert_eq!(
        reported["lint"]["select"],
        serde_json::json!(["LNK", "DUP", "PTR", "KND001"])
    );
    assert!(reported["lint"].get("extend-select").is_none());
    assert!(reported.get("extend-exclude").is_none());

    let overrides = CliOverrides {
        select: Some(vec!["ORD".into()]),
        extend_select: vec!["PTR".into()],
        preview: true,
    };
    let selected = config
        .selected_rules(Path::new("docs/page.md"), &overrides)
        .unwrap();
    assert!(selected.iter().any(|code| code.starts_with("ORD")));
    assert!(selected.iter().any(|code| code.starts_with("PTR")));
    assert!(!selected.iter().any(|code| code.starts_with("LNK")));
}

#[test]
fn invalid_extension_selectors_are_rejected() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "seiso.toml",
        "[lint]\nextend-select = ['NOT_A_RULE']\n",
    );
    let error = Config::load(&dir.path().join("seiso.toml")).unwrap_err();
    assert!(matches!(error, ConfigError::Invalid { .. }));
    assert!(error.to_string().contains("NOT_A_RULE"));
}

#[test]
fn invalid_inherited_extension_list_is_not_hidden_by_child_array() {
    let dir = tempdir().unwrap();
    write(dir.path(), "base.toml", "[lint]\nextend-select = 'DUP'\n");
    write(
        dir.path(),
        "child/seiso.toml",
        "extend = '../base.toml'\n[lint]\nextend-select = ['PTR']\n",
    );

    let error = Config::load(&dir.path().join("child/seiso.toml"))
        .map(|_| ())
        .expect_err("an invalid inherited extension list must be rejected");
    assert!(matches!(error, ConfigError::Invalid { .. }));
    assert!(
        error
            .to_string()
            .contains("extend-select must be an array of strings")
    );
}

#[test]
fn recursive_extend_paths_use_each_declaring_directory() {
    let dir = tempdir().unwrap();
    write(dir.path(), "shared/base.toml", "preview = true");
    write(
        dir.path(),
        "nested/base.toml",
        "extend = '../shared/base.toml'\n[lint]\nignore = ['DUP']",
    );
    write(
        dir.path(),
        "docs/seiso.toml",
        "extend = '../nested/base.toml'",
    );
    let config = Config::load(&dir.path().join("docs/seiso.toml")).unwrap();
    assert!(config.settings.preview);
    assert_eq!(config.settings.lint.ignore, ["DUP"]);
}

/// Extending a governing parent preserves every inherited path-based policy.
#[test]
fn inherited_parent_patterns_keep_their_original_bases() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
include = ['docs/**/*.md']
exclude = ['docs/private/**']
[[kinds]]
path = 'docs/reference/**'
kind = 'reference'
[[domains]]
path = 'docs/**'
name = 'manual'
[[sites]]
path = 'docs/**'
root = 'docs'
public = 'docs/public'
[lint]
select = ['KND', 'LNK']
[lint.per-file-ignores]
'docs/reference/**' = ['LNK']
"#,
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    let path = Path::new("reference/config.md");
    assert!(config.includes(path));
    assert!(config.excludes(Path::new("private/secret.md")));
    assert_eq!(config.kind_for(path), Some(Kind::Reference));
    assert_eq!(config.domain_for(path), Some("manual"));
    assert_eq!(config.site_for(path).unwrap().root, "docs");
    assert_eq!(
        config
            .selected_rules(path, &CliOverrides::default())
            .unwrap(),
        ["KND001", "KND002"]
    );
    let bases = config.pattern_bases(&root.join("docs"));
    for field in [
        "include",
        "exclude",
        "kinds",
        "domains",
        "sites",
        "lint.per-file-ignores",
    ] {
        assert_eq!(bases[field][0].base_directory, "..", "{field}");
    }
}

/// A template adopts the extending parent's base, which survives later inheritance.
#[test]
fn mixed_template_and_parent_chains_preserve_per_entry_origins() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "shared/template.toml",
        "include = ['docs/**/*.md']\n[lint.per-file-ignores]\n'docs/old/**' = ['LNK']",
    );
    write(
        root,
        "seiso.toml",
        "extend = 'shared/template.toml'\nexclude = ['docs/private/**']",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../seiso.toml'\nexclude = ['local/**']\n[lint.per-file-ignores]\n'reference/**' = ['KND']",
    );
    write(
        root,
        "docs/reference/seiso.toml",
        "extend = '../seiso.toml'",
    );
    let config = Config::load(&root.join("docs/reference/seiso.toml")).unwrap();
    assert!(config.includes(Path::new("a.md")));
    assert_eq!(
        config
            .selected_rules(Path::new("a.md"), &CliOverrides::default())
            .unwrap(),
        ["LNK001", "SUP001", "SUP002"]
    );
    let bases = config.pattern_bases(root);
    assert_eq!(bases["include"][0].base_directory, ".");
    assert_eq!(bases["exclude"][0].base_directory, "docs");
    assert_eq!(bases["lint.per-file-ignores"][0].base_directory, ".");
    assert_eq!(bases["lint.per-file-ignores"][1].base_directory, "docs");
}

/// Shared bases and non-governing files keep the existing caller-relative semantics.
#[test]
fn non_governing_bases_are_reusable_at_the_selected_directory() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, ".seiso.toml", "");
    write(root, "seiso.toml", "include = ['reference/*.md']");
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert!(config.includes(Path::new("reference/a.md")));
    assert_eq!(
        config.pattern_bases(root)["include"][0].base_directory,
        "docs"
    );
    write(
        root,
        "docs/base/template.toml",
        "include = ['reference/*.md']",
    );
    write(root, "docs/seiso.toml", "extend = 'base/template.toml'");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert!(config.includes(Path::new("reference/a.md")));
}

/// Governing pyproject tables retain their base; explicit config still rebases own entries.
#[test]
fn pyproject_parents_and_explicit_config_have_distinct_bases() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "pyproject.toml",
        "[tool.seiso]\nexclude = ['docs/private/**']",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../pyproject.toml'\ninclude = ['docs/**/*.md']",
    );
    let config = Config::load_from(&root.join("docs/seiso.toml"), root).unwrap();
    assert!(config.includes(Path::new("docs/page.md")));
    assert!(config.excludes(Path::new("docs/private/page.md")));
    assert_eq!(config.pattern_bases(root)["include"][0].base_directory, ".");
    let ordinary = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert!(!ordinary.includes(Path::new("page.md")));
    assert!(ordinary.excludes(Path::new("private/page.md")));
}

#[test]
fn inheritance_cycles_report_the_files() {
    let dir = tempdir().unwrap();
    write(dir.path(), "first.toml", "extend = 'sub/../second.toml'");
    write(dir.path(), "second.toml", "extend = 'first.toml'");
    let error = Config::load(&dir.path().join("first.toml")).unwrap_err();
    assert!(matches!(error, ConfigError::Cycle(_)));
    assert!(error.to_string().contains("first.toml"));
    assert!(error.to_string().contains("second.toml"));
}

#[cfg(unix)]
#[test]
fn inheritance_cycles_resolve_symlink_aliases() {
    let dir = tempdir().unwrap();
    write(dir.path(), "first.toml", "extend = 'alias.toml'");
    std::os::unix::fs::symlink(dir.path().join("first.toml"), dir.path().join("alias.toml"))
        .unwrap();
    assert!(matches!(
        Config::load(&dir.path().join("first.toml")),
        Err(ConfigError::Cycle(_))
    ));
}

#[test]
fn excessively_deep_inheritance_returns_an_error() {
    let dir = tempdir().unwrap();
    for index in 0..130 {
        write(
            dir.path(),
            &format!("{index}.toml"),
            &format!("extend = '{}.toml'", index + 1),
        );
    }
    write(dir.path(), "130.toml", "");
    let error = Config::load(&dir.path().join("0.toml"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("inheritance exceeds"));
}

#[test]
fn pyproject_extension_extracts_only_tool_seiso() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "pyproject.toml",
        "[project]\nname = 'test'\n[tool.seiso]\npreview = true",
    );
    write(
        dir.path(),
        "docs/seiso.toml",
        "extend = '../pyproject.toml'",
    );
    assert!(
        Config::load(&dir.path().join("docs/seiso.toml"))
            .unwrap()
            .settings
            .preview
    );
}

#[test]
fn exact_selection_beats_family_ignore_and_ties_favor_ignore() {
    let (_dir, config) = parse(
        "preview = true\n[lint]\nselect = ['KND001', 'KND002', 'STL']\nignore = ['KND', 'KND002', 'STL001']",
    );
    assert_eq!(
        rules(&config, Some("howto"), &CliOverrides::default()),
        ["KND001", "STL002", "STL003", "STL004"]
    );
}

#[test]
fn all_has_lower_specificity_than_family() {
    let (_dir, config) = parse("preview = true\n[lint]\nselect = ['KND']\nignore = ['ALL']");
    assert_eq!(
        rules(&config, Some("howto"), &CliOverrides::default()),
        ["KND001", "KND002"]
    );
    let (_dir, config) = parse("preview = true\n[lint]\nselect = ['ALL']\nignore = ['ALL']");
    assert!(rules(&config, Some("howto"), &CliOverrides::default()).is_empty());
}

#[test]
fn cli_select_replaces_and_extend_select_appends() {
    let (_dir, config) = parse("[lint]\nselect = ['DUP']\nignore = ['KND']");
    let overrides = CliOverrides {
        select: Some(vec!["LNK".into()]),
        extend_select: vec!["KND001".into()],
        preview: true,
    };
    assert_eq!(
        rules(&config, Some("howto"), &overrides),
        ["KND001", "LNK001", "LNK002"]
    );
}

#[test]
fn accepted_consistency_rules_are_default_and_other_rules_require_preview() {
    let (_dir, config) = parse("");
    assert_eq!(
        rules(&config, Some("howto"), &CliOverrides::default()),
        ["KND001", "KND002", "LNK001", "SUP001", "SUP002"]
    );
    let preview = rules(
        &config,
        Some("howto"),
        &CliOverrides {
            preview: true,
            ..Default::default()
        },
    );
    assert_eq!(preview.len(), rule_codes().count());
    for code in rule_codes() {
        assert!(preview.contains(&code));
    }
}

#[test]
fn per_file_ignores_apply_after_exact_selection() {
    let (_dir, config) = parse(
        "preview = true\n[lint]\nselect = ['DUP001', 'LNK']\n[lint.per-file-ignores]\n'docs/**' = ['DUP']\n'docs/page.md' = ['LNK001']",
    );
    assert_eq!(
        rules(&config, Some("howto"), &CliOverrides::default()),
        ["LNK002"]
    );
    assert_eq!(
        config
            .enabled_rules(
                Path::new("README.md"),
                Some(seiso::rules::Kind::Readme),
                &CliOverrides::default()
            )
            .unwrap(),
        ["DUP001", "LNK001", "LNK002"]
    );
}

#[test]
fn kind_controls_rule_applicability() {
    let (_dir, config) = parse("preview = true");
    assert_eq!(
        rules(&config, None, &CliOverrides::default()),
        ["KND001", "KND002", "LNK001", "LNK002", "SUP001", "SUP002"]
    );
    assert!(rules(&config, Some("generated"), &CliOverrides::default()).is_empty());
    let changelog = rules(&config, Some("changelog"), &CliOverrides::default());
    assert!(!changelog.contains(&"STL001"));
    assert!(!changelog.contains(&"PTR001"));
    assert!(changelog.contains(&"PTR003"));
    let reference = rules(&config, Some("reference"), &CliOverrides::default());
    assert!(reference.contains(&"RAT002"));
    let readme = rules(&config, Some("readme"), &CliOverrides::default());
    assert!(!readme.contains(&"RAT002"));
    let agents = rules(&config, Some("agents"), &CliOverrides::default());
    for code in [
        "STL001", "STL002", "STL003", "STL004", "VOX001", "VOX002", "VOX003", "KND001", "LNK001",
        "SUP001", "PTR001", "EVD001", "DUP001", "OWN002",
    ] {
        assert!(agents.contains(&code), "{code} should apply to agents");
    }
    for code in ["RAT001", "RAT002", "ORD001", "ORD002", "MIX001"] {
        assert!(!agents.contains(&code), "{code} should not apply to agents");
    }
}

#[test]
fn agents_kind_is_accepted_and_serialized_in_configuration() {
    let (_dir, config) = parse("[[kinds]]\npath = '**/AGENTS.md'\nkind = 'agents'");
    assert_eq!(config.settings.kinds[0].kind, "agents");
    let json = serde_json::to_value(&config.settings).unwrap();
    assert_eq!(json["kinds"][0]["kind"], "agents");
}

#[test]
fn dependency_selection_precedes_document_kind_applicability() {
    let (_dir, config) = parse(
        "preview = true\n[lint]\nselect = ['STL', 'DUP', 'KND']\nignore = ['DUP']\n[lint.per-file-ignores]\n'docs/**' = ['KND']",
    );
    assert_eq!(
        config
            .selected_rules(Path::new("docs/page.md"), &CliOverrides::default())
            .unwrap(),
        ["STL001", "STL002", "STL003", "STL004"]
    );
    assert!(rules(&config, None, &CliOverrides::default()).is_empty());
    assert!(rules(&config, Some("generated"), &CliOverrides::default()).is_empty());
}

#[test]
fn unimplemented_rules_are_rejected_in_every_selection_surface() {
    let dir = tempdir().unwrap();
    let config = Config::defaults(dir.path()).unwrap();
    for code in ["STL999", "RAT999", "ORD999", "MIX999", "VOX999", "EVD999"] {
        for preview in [false, true] {
            let overrides = CliOverrides {
                select: Some(vec![code.into()]),
                preview,
                ..Default::default()
            };
            assert!(matches!(
                config.enabled_rules(
                    Path::new("page.md"),
                    Some(seiso::rules::Kind::Howto),
                    &overrides
                ),
                Err(ConfigError::Selector(_))
            ));
            for selection in [
                format!("[lint]\nselect = ['{code}']"),
                format!("[lint]\nignore = ['{code}']"),
                format!("[lint.per-file-ignores]\n'**' = ['{code}']"),
            ] {
                assert!(
                    Config::parse(&format!("preview = {preview}\n{selection}"), dir.path())
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn invalid_values_name_the_setting_and_source() {
    let dir = tempdir().unwrap();
    for (source, expected) in [
        ("unknown = true", "unknown"),
        ("[lint]\nunknown = true", "unknown"),
        ("[[kinds]]\npath = '**'\nkind = 'tutorial'", "tutorial"),
        ("[[domains]]\npath = '**'\nname = ''", "domains.name"),
        ("include = ['[']", "include"),
        ("exclude = ['']", "exclude"),
        ("[lint]\nselect = ['KND999']", "KND999"),
        ("[lint]\nignore = ['K']", "selector"),
        ("[lint]\nlanguages = ['fr']", "fr"),
        ("[lint.lexicon.fr]\nextend-stale-markers = ['now']", "fr"),
        (
            "[lint.lexicon.zh]\nextend-stale-markers = ['']",
            "extend-stale-markers",
        ),
        ("[lint.dup]\nmin-identifiers = 0", "min-identifiers"),
        ("[lint.dup]\nmin-jaccard = 1.1", "min-jaccard"),
        ("[lint.dup]\nmin-jaccard = nan", "min-jaccard"),
        (
            "[lint.dup]\nmin-paragraph-similarity = 0.0",
            "min-paragraph-similarity",
        ),
        (
            "[lint.dup]\nmin-paragraph-similarity = 1.1",
            "min-paragraph-similarity",
        ),
        (
            "[lint.dup]\nmin-paragraph-similarity = nan",
            "min-paragraph-similarity",
        ),
        ("[lint.dup]\nmin-paragraph-chars = 0", "min-paragraph-chars"),
        ("[lint.dup]\nshingle-size = 0", "shingle-size"),
        ("[lint.ptr]\ncatalog-dirs = ['']", "catalog-dirs"),
        ("[lint.per-file-ignores]\n'[' = ['ALL']", "glob"),
        ("[lint.per-file-ignores]\n'**' = ['UNKNOWN']", "UNKNOWN"),
    ] {
        let error = Config::parse(source, dir.path()).unwrap_err().to_string();
        assert!(error.contains(expected), "{source}: {error}");
        assert!(error.contains("seiso.toml"), "{error}");
    }
}

#[test]
fn invalid_cli_selector_is_rejected_even_without_preview() {
    let (_dir, config) = parse("");
    let invalid = CliOverrides {
        select: Some(vec!["WRONG".into()]),
        ..Default::default()
    };
    assert!(matches!(
        config.enabled_rules(
            Path::new("page.md"),
            Some(seiso::rules::Kind::Howto),
            &invalid
        ),
        Err(ConfigError::Selector(_))
    ));
}

#[test]
fn missing_extend_and_invalid_config_fail_loudly() {
    let dir = tempdir().unwrap();
    write(dir.path(), "seiso.toml", "extend = 'missing.toml'");
    assert!(
        Config::load(&dir.path().join("seiso.toml"))
            .unwrap_err()
            .to_string()
            .contains("missing.toml")
    );
    write(dir.path(), "seiso.toml", "extend = []");
    assert!(
        Config::load(&dir.path().join("seiso.toml"))
            .unwrap_err()
            .to_string()
            .contains("extend")
    );
    write(dir.path(), "seiso.toml", "preview = '");
    assert!(Workspace::discover(dir.path(), None).is_err());
}

#[test]
fn documented_lexicon_and_threshold_fields_load() {
    let (_dir, config) = parse(
        r#"
[lint]
languages = ["en", "zh", "ja"]
[lint.dup]
min-identifiers = 5
min-jaccard = 0.8
min-paragraph-similarity = 0.95
min-paragraph-chars = 100
shingle-size = 7
[lint.ptr]
catalog-dirs = ["locales/", "migrations/"]
[lint.lexicon.zh]
extend-stale-markers = ["截至目前"]
"#,
    );
    assert_eq!(
        config.settings.lint.lexicon["zh"].extend_stale_markers,
        ["截至目前"]
    );
    assert_eq!(
        config.settings.lint.ptr.catalog_dirs,
        ["locales/", "migrations/"]
    );
    assert_eq!(config.settings.lint.dup.min_paragraph_similarity, 0.95);
    assert_eq!(config.settings.lint.dup.min_paragraph_chars, 100);
    assert_eq!(config.settings.lint.dup.shingle_size, 7);
}

#[test]
fn inherited_additive_exclusions_keep_each_declaring_base() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "exclude = ['unused/**']\nextend-exclude = ['docs/api/generated/**']",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../seiso.toml'\nexclude = ['api/private/**']\nextend-exclude = ['api/legacy/**']",
    );
    write(
        root,
        "docs/api/seiso.toml",
        "extend = '../seiso.toml'\nextend-exclude = ['draft/**']",
    );
    let config = Config::load(&root.join("docs/api/seiso.toml")).unwrap();
    for path in [
        "private/a.md",
        "generated/a.md",
        "legacy/a.md",
        "draft/a.md",
    ] {
        assert!(config.excludes(Path::new(path)), "{path}");
    }
    assert!(!config.excludes(Path::new("unused/a.md")));
    let bases = config.pattern_bases(root);
    assert_eq!(
        bases["exclude"]
            .iter()
            .map(|entry| entry.base_directory.as_str())
            .collect::<Vec<_>>(),
        ["docs", ".", "docs", "docs/api"]
    );
}

#[test]
fn governing_inheritance_keeps_workspace_even_without_inherited_path_entries() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, "seiso.toml", "");
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    write(root, "docs/guide/seiso.toml", "extend = '../seiso.toml'\n");
    let workspace = Workspace::discover(&root.join("docs/guide"), None).unwrap();
    assert_eq!(workspace.root, root);
    assert_eq!(
        workspace
            .config_for(&root.join("docs/guide/page.md"))
            .unwrap()
            .directory,
        root.join("docs/guide")
    );
    assert_eq!(
        workspace
            .config_for(&root.join("index.md"))
            .unwrap()
            .directory,
        root
    );
}

#[test]
fn extending_a_template_does_not_expand_the_workspace() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(root, ".seiso.toml", "");
    write(root, "seiso.toml", "");
    // The shadowed file is a template, not the governing ancestor.
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    let workspace = Workspace::discover(&root.join("docs"), None).unwrap();
    assert_eq!(workspace.root, root.join("docs"));
}

#[test]
fn mapping_extensions_accumulate_after_replacements_with_individual_bases() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
extend-exclude = ['docs/hidden/**']
[[kinds]]
path = '**'
kind = 'readme'
[[extend-kinds]]
path = 'docs/**'
kind = 'reference'
[[extend-domains]]
path = 'docs/**'
name = 'parent'
[[extend-sites]]
path = 'docs/**'
root = 'docs'
"#,
    );
    write(
        root,
        "docs/seiso.toml",
        r#"
extend = '../seiso.toml'
extend-exclude = ['private/**']
[[kinds]]
path = '**'
kind = 'howto'
[[domains]]
path = '**'
name = 'replacement'
[[sites]]
path = '**'
root = '.'
[[extend-kinds]]
path = 'api/**'
kind = 'generated'
[[extend-domains]]
path = 'api/**'
name = 'child'
[[extend-sites]]
path = 'api/**'
root = 'api'
"#,
    );
    write(
        root,
        "docs/api/seiso.toml",
        r#"
extend = '../seiso.toml'
[[extend-kinds]]
path = 'special.md'
kind = 'plan'
[[extend-domains]]
path = 'special.md'
name = 'grandchild'
[[extend-sites]]
path = 'special.md'
root = '.'
base = '/special/'
"#,
    );
    let config = Config::load(&root.join("docs/api/seiso.toml")).unwrap();
    assert_eq!(
        config
            .settings
            .kinds
            .iter()
            .map(|m| m.kind.as_str())
            .collect::<Vec<_>>(),
        ["howto", "reference", "generated", "plan"]
    );
    assert_eq!(config.kind_for(Path::new("page.md")), Some(Kind::Generated));
    assert_eq!(config.kind_for(Path::new("special.md")), Some(Kind::Plan));
    assert_eq!(config.domain_for(Path::new("page.md")), Some("child"));
    assert_eq!(
        config.domain_for(Path::new("special.md")),
        Some("grandchild")
    );
    assert_eq!(config.site_for(Path::new("page.md")).unwrap().root, "api");
    assert_eq!(
        config.site_for(Path::new("special.md")).unwrap().base,
        "/special/"
    );
    let bases = config.pattern_bases(root);
    assert_eq!(
        bases["kinds"]
            .iter()
            .map(|p| p.base_directory.as_str())
            .collect::<Vec<_>>(),
        ["docs", ".", "docs", "docs/api"]
    );
    assert_eq!(
        bases["exclude"]
            .iter()
            .map(|p| p.base_directory.as_str())
            .collect::<Vec<_>>(),
        [".", "docs"]
    );
    let child = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert!(child.excludes(Path::new("hidden/a.md")));
    assert!(child.excludes(Path::new("private/a.md")));
    assert_eq!(child.kind_for(Path::new("guide.md")), Some(Kind::Reference));
}

#[test]
fn mapping_extensions_validate_entries_and_globs() {
    let dir = tempdir().unwrap();
    for source in [
        "extend-kinds = 'wrong'",
        "[[extend-kinds]]\npath = '**'\nkind = 'unknown'",
        "[[extend-kinds]]\npath = '['\nkind = 'reference'",
        "[[extend-domains]]\npath = '**'\nname = ''",
        "[[extend-domains]]\npath = '**'\nunknown = 'x'",
        "[[extend-sites]]\npath = '**'\nroot = '/absolute'",
        "[[extend-sites]]\npath = '**'\nroot = '.'\nbase = 'invalid'",
        "[[extend-sites]]\npath = '['\nroot = '.'",
    ] {
        assert!(Config::parse(source, dir.path()).is_err(), "{source}");
    }
}
