use std::fs;
use std::path::Path;

use seiso::config::{CliOverrides, Config, ConfigError, Workspace};
use seiso::rules::rule_codes;
use tempfile::{TempDir, tempdir};

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn parse(source: &str) -> (TempDir, Config) {
    let dir = tempdir().unwrap();
    let config = Config::parse(source, dir.path()).unwrap();
    (dir, config)
}

fn rules(config: &Config, kind: Option<&str>, overrides: &CliOverrides) -> Vec<&'static str> {
    config
        .enabled_rules(Path::new("docs/page.md"), kind, overrides)
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
    assert_eq!(config.kind_for(Path::new("docs/intro.md")), Some("howto"));
    assert_eq!(
        config.kind_for(Path::new("docs/reference/config.md")),
        Some("reference")
    );
    assert_eq!(
        config.kind_for(Path::new("docs/reference/generated/client.md")),
        Some("generated")
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
        Some("reference")
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
        Some("reference")
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
                Some("readme"),
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
                config.enabled_rules(Path::new("page.md"), Some("howto"), &overrides),
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
        config.enabled_rules(Path::new("page.md"), Some("howto"), &invalid),
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
