use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn run(root: &Path, arguments: &[&str], input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(root)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn value(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn workspace() -> TempDir {
    let root = TempDir::new().unwrap();
    write(root.path(), "seiso.toml", "");
    root
}

#[test]
fn preview_is_opt_in_even_when_a_rule_is_explicitly_selected() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    write(
        root,
        "a.md",
        "---\nkind: howto\n---\n# Setup\n\nCurrently uses v1.2.3.\n",
    );
    let disabled = run(
        root,
        &["check", "--select", "STL001", "--output-format", "json"],
        None,
    );
    assert_eq!(disabled.status.code(), Some(0));
    assert_eq!(value(&disabled), json!([]));
    assert!(String::from_utf8_lossy(&disabled.stderr).contains("No rules enabled"));
    let enabled = run(
        root,
        &[
            "check",
            "--preview",
            "--select",
            "STL001",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(enabled.status.code(), Some(1));
    assert_eq!(value(&enabled)[0]["code"], "STL001");
    assert!(enabled.stderr.is_empty());
}

#[test]
fn stable_rules_run_by_default_and_respect_selection_ignores_and_generated() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "a.md", "# Missing kind\n\n[Missing](missing.md)\n");
    let output = run(root, &["check", "--output-format", "json"], None);
    assert_eq!(output.status.code(), Some(1));
    let codes: Vec<String> = value(&output)
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, ["KND001", "LNK001"]);
    assert!(output.stderr.is_empty());
    let selected = run(
        root,
        &["check", "--select", "LNK", "--output-format", "json"],
        None,
    );
    assert_eq!(value(&selected).as_array().unwrap().len(), 1);
    assert_eq!(value(&selected)[0]["code"], "LNK001");
    write(root, "seiso.toml", "[lint]\nignore = ['KND', 'LNK']\n");
    let ignored = run(root, &["check", "--output-format", "json"], None);
    assert_eq!(ignored.status.code(), Some(0));
    assert_eq!(value(&ignored), json!([]));
    write(
        root,
        "seiso.toml",
        "[[kinds]]\npath = 'a.md'\nkind = 'generated'\n",
    );
    let generated = run(root, &["check", "--output-format", "json"], None);
    assert_eq!(generated.status.code(), Some(0));
    assert_eq!(value(&generated), json!([]));
}

#[test]
fn selected_reports_are_deterministic_subsets_of_full_reports() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "a.md", "# A\n");
    write(root, "nested/b.md", "# B\n");
    let full = run(root, &["check", "--output-format", "json"], None);
    let repeated = run(
        root,
        &[
            "check",
            "--output-format",
            "json",
            "--no-cache",
            "nested/b.md",
            "a.md",
            "a.md",
        ],
        None,
    );
    assert_eq!(full.status.code(), Some(1));
    assert_eq!(full.stdout, repeated.stdout);
    let nested = run(
        &root.join("nested"),
        &["check", "--output-format", "json", "b.md"],
        None,
    );
    let expected: Vec<Value> = value(&full)
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["filename"] == "nested/b.md")
        .cloned()
        .collect();
    assert_eq!(value(&nested), json!(expected));
}

#[test]
fn incomplete_checks_preserve_diagnostics_and_override_exit_zero() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "good.md", "# Missing kind\n");
    std::fs::write(root.join("unreadable.md"), [0xff, 0xfe]).unwrap();
    let output = run(
        root,
        &["check", "--exit-zero", "--output-format", "json"],
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(value(&output)[0]["filename"], "good.md");
    assert!(String::from_utf8_lossy(&output.stderr).contains("unreadable.md"));
    let selected = run(
        root,
        &["check", "good.md", "--exit-zero", "--output-format", "json"],
        None,
    );
    assert_eq!(selected.status.code(), Some(0));
    assert!(selected.stderr.is_empty());
    assert_eq!(value(&output), value(&selected));
    let missing = run(
        root,
        &[
            "check",
            "good.md",
            "missing.md",
            "--exit-zero",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(value(&missing), value(&selected));
}

#[test]
fn local_checks_and_stdin_ignore_unrelated_source_and_configuration_errors() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "good.md", "# Original\n");
    write(root, "broken-config/seiso.toml", "unexpected = true\n");
    write(root, "broken-config/other.md", "# Other\n");
    std::fs::write(root.join("unreadable.md"), [0xff]).unwrap();
    let args = [
        "check",
        "good.md",
        "--select",
        "KND",
        "--no-cache",
        "--output-format",
        "json",
    ];
    let selected = run(root, &args, None);
    assert_eq!(selected.status.code(), Some(1));
    assert!(selected.stderr.is_empty());
    assert_eq!(value(&selected)[0]["code"], "KND001");
    let stdin = run(
        root,
        &[
            "check",
            "--stdin-filename",
            "good.md",
            "--select",
            "KND",
            "--no-cache",
            "--output-format",
            "json",
        ],
        Some("---\nkind: reference\n---\n# Buffer\n"),
    );
    assert_eq!(stdin.status.code(), Some(0));
    assert!(stdin.stderr.is_empty());
    assert_eq!(value(&stdin), json!([]));
    assert_eq!(
        std::fs::read_to_string(root.join("good.md")).unwrap(),
        "# Original\n"
    );
    let full = run(
        root,
        &["check", "--select", "KND", "--output-format", "json"],
        None,
    );
    assert_eq!(full.status.code(), Some(2));
    let errors = String::from_utf8_lossy(&full.stderr);
    assert!(errors.contains("unreadable.md"));
    assert!(errors.contains("broken-config/other.md"));
}

#[test]
fn local_checks_ignore_unrelated_ignore_pattern_errors() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "good.md", "---\nkind: reference\n---\n# Good\n");
    write(root, "other/.gitignore", "[z-a]\n");
    write(root, "other/other.md", "# Other\n");
    let selected = run(
        root,
        &[
            "check",
            "good.md",
            "--select",
            "KND",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(
        selected.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert!(selected.stderr.is_empty());
    let full = run(
        root,
        &["check", "--select", "KND", "--output-format", "json"],
        None,
    );
    assert_eq!(full.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&full.stderr).contains("ignore"));
}

#[test]
fn explicit_paths_obey_excludes_gitignore_and_nearest_configuration() {
    let workspace = workspace();
    let root = workspace.path();
    write(
        root,
        "seiso.toml",
        "preview = true\nexclude = ['skip.md', 'docs/**']\n",
    );
    write(root, ".gitignore", "ignored.md\n");
    write(root, "skip.md", "# Skipped\n");
    write(root, "ignored.md", "# Ignored\n");
    write(
        root,
        "docs/.seiso.toml",
        "preview = true\n[[kinds]]\npath = '*.md'\nkind = 'reference'\n",
    );
    write(root, "docs/valid.md", "# Valid\n");
    write(root, "docs/invalid.md", "---\nkind: nope\n---\n# Invalid\n");
    let output = run(
        root,
        &[
            "check",
            "skip.md",
            "ignored.md",
            "docs",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(diagnostics[0]["filename"], "docs/invalid.md");
    assert_eq!(diagnostics[0]["code"], "KND002");
    let ignored_stdin = run(
        root,
        &[
            "check",
            "--stdin-filename",
            "ignored.md",
            "--output-format",
            "json",
        ],
        Some("# Input"),
    );
    assert_eq!(ignored_stdin.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ignored_stdin.stderr).contains(".gitignore"));
}

#[test]
fn stdin_replaces_content_uses_its_path_and_leaves_disk_unchanged() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "docs/a.md", "# Original\n");
    write(
        root,
        "docs/target.md",
        "---\nkind: reference\n---\n# Target\n",
    );
    write(
        root,
        "docs/.seiso.toml",
        "preview = true\n[[kinds]]\npath = '*.md'\nkind = 'reference'\n",
    );
    let output = run(
        root,
        &[
            "check",
            "--stdin-filename",
            "docs/a.md",
            "--output-format",
            "json",
        ],
        Some("# Replacement\n\n[Target](target.md)\n"),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(value(&output), json!([]));
    assert_eq!(
        std::fs::read_to_string(root.join("docs/a.md")).unwrap(),
        "# Original\n"
    );
    let new_file = run(
        root,
        &[
            "check",
            "--stdin-filename",
            "docs/new.md",
            "--output-format",
            "json",
        ],
        Some("# New\n\n[Missing](missing.md)\n"),
    );
    assert_eq!(new_file.status.code(), Some(1));
    assert_eq!(value(&new_file)[0]["code"], "LNK001");
    assert!(!root.join("docs/new.md").exists());
}

#[test]
fn new_stdin_documents_resolve_self_and_workspace_root_links() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "README.md", "---\nkind: readme\n---\n# Project\n");
    write(root, "space file.md", "# Target\n");
    std::fs::create_dir(root.join("docs")).unwrap();
    let source = "---\nkind: reference\n---\n# New\n\n[Self](new.md)\n\n[Self from root](/docs/new.md)\n\n[Project](/README.md)\n\n[Encoded](/space%20file.md)\n";
    let output = run(
        root,
        &[
            "check",
            "--stdin-filename",
            "docs/new.md",
            "--output-format",
            "json",
        ],
        Some(source),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(value(&output), json!([]));
    assert!(output.stderr.is_empty());
    assert!(!root.join("docs/new.md").exists());
    let nested = run(
        &root.join("docs"),
        &[
            "check",
            "--stdin-filename",
            "new.md",
            "--output-format",
            "json",
        ],
        Some(source),
    );
    assert_eq!(nested.status.code(), Some(0));
    assert_eq!(nested.stdout, output.stdout);
}

#[test]
fn disabled_preview_rules_do_not_execute_or_stale_their_suppressions() {
    let workspace = workspace();
    let root = workspace.path();
    write(
        root,
        "a.md",
        "---\nkind: reference\n---\n<!-- seiso: allow LNK002 -- The generated heading is added later. -->\n[Heading](#missing)\n",
    );
    let output = run(
        root,
        &[
            "check",
            "--select",
            "LNK002",
            "--extend-select",
            "SUP002",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(value(&output), json!([]));
    let policy = run(
        root,
        &["policy", "--select", "LNK002", "--extend-select", "SUP002"],
        None,
    );
    let report = value(&policy);
    assert_eq!(report["files"][0]["enabled_rules"], json!(["SUP002"]));
    assert_eq!(
        report["files"][0]["suppressions"][0]["states"]["LNK002"]["state"],
        "rule_disabled"
    );
}

#[test]
fn policy_paths_are_workspace_relative_and_independent_of_calling_directory() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "docs/nested/a.md", "---\nkind: howto\n---\n# Guide\n");
    write(
        root,
        "docs/.seiso.toml",
        "extend = '../seiso.toml'\nexclude = ['excluded.md']\n",
    );
    write(root, "docs/excluded.md", "# Excluded\n");
    std::fs::create_dir(root.join("caller")).unwrap();
    let output = run(root, &["policy"], None);
    let nested = run(&root.join("caller"), &["policy"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(nested.stdout, output.stdout);
    let report = value(&output);
    assert_eq!(report["files"][1]["filename"], "docs/nested/a.md");
    assert_eq!(report["files"][1]["configuration"], "docs/.seiso.toml");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&root.to_string_lossy().to_string()));
}

#[test]
fn malformed_arguments_and_outside_stdin_paths_return_tool_errors() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "a.md", "# A\n");
    for arguments in [
        vec!["check", "--select", "NOPE"],
        vec!["check", "--stdin-filename", "a.md", "a.md"],
        vec!["check", "--stdin-filename", "../outside.md"],
        vec!["check", "--stdin-filename", "a.rs"],
        vec!["check", "--config", "missing.toml"],
        vec!["rule"],
        vec!["rule", "--all", "KND001"],
        vec!["hook", "unknown-adapter"],
    ] {
        let output = run(root, &arguments, None);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
}

#[test]
fn symlinked_external_directories_are_not_traversed_or_reported_as_missing() {
    let workspace = workspace();
    let root = workspace.path();
    let external = TempDir::new().unwrap();
    write(external.path(), "outside.md", "# Missing kind\n");
    let link = root.join("external");
    #[cfg(unix)]
    std::os::unix::fs::symlink(external.path(), &link).unwrap();
    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(external.path(), &link) {
        Ok(()) => {}
        Err(error)
            if error.raw_os_error() == Some(1314)
                || error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            eprintln!(
                "Skipping directory symlink test: Windows symlink permission is unavailable."
            );
            return;
        }
        Err(error) => panic!("Cannot create test symlink: {error}"),
    }
    write(
        root,
        "a.md",
        "---\nkind: reference\n---\n<!-- seiso: allow LNK001 -- The external checkout owns this target. -->\n[External](external/not-there.md)\n",
    );
    let output = run(root, &["check", "--output-format", "json"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(value(&output), json!([]));
    let policy = run(root, &["policy", "--evaluate"], None);
    let report = value(&policy);
    assert_eq!(report["files"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["files"][0]["suppressions"][0]["states"]["LNK001"]["state"],
        "incomplete"
    );
}

#[test]
fn policy_exposes_effective_overrides_exclusions_generated_and_suppressions() {
    let workspace = workspace();
    let root = workspace.path();
    write(
        root,
        "seiso.toml",
        "exclude = ['skip.md']\n[[kinds]]\npath = 'generated.md'\nkind = 'generated'\n",
    );
    write(
        root,
        "generated.md",
        "# Generated\n\n[Missing](missing.md)\n",
    );
    write(root, "skip.md", "# Excluded\n");
    write(
        root,
        "a.md",
        "---\nkind: reference\n---\n<!-- seiso: allow-file LNK001 -- The generator creates this page. -->\n\n[Missing](missing.md)\n",
    );
    let args = [
        "policy",
        "--preview",
        "--select",
        "LNK",
        "--extend-select",
        "SUP",
    ];
    let output = run(root, &args, None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, run(root, &args, None).stdout);
    let report = value(&output);
    assert_eq!(report["configurations"]["seiso.toml"]["preview"], true);
    assert_eq!(
        report["configurations"]["seiso.toml"]["lint"]["select"],
        json!(["LNK", "SUP"])
    );
    assert_eq!(report["files"][0]["kind"]["value"], "reference");
    assert_eq!(
        report["files"][0]["suppressions"][0]["reason"],
        "The generator creates this page."
    );
    assert_eq!(
        report["files"][0]["suppressions"][0]["states"]["LNK001"]["state"],
        "not_evaluated"
    );
    let mut evaluated_args = args.to_vec();
    evaluated_args.push("--evaluate");
    let evaluated = run(root, &evaluated_args, None);
    assert_eq!(evaluated.status.code(), Some(0));
    assert_eq!(
        value(&evaluated)["files"][0]["suppressions"][0]["states"]["LNK001"],
        json!({"state":"active", "count":1})
    );
    assert_eq!(report["files"][1]["kind"]["value"], "generated");
    assert_eq!(report["files"][1]["enabled_rules"], json!([]));
    assert_eq!(report["files"][2]["excluded"], "exclude");
}

#[test]
fn init_preserves_existing_configuration_and_only_maps_known_directories() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    write(root, "README.md", "# Intro\n");
    write(root, "docs/reference/config.md", "# Options\n");
    write(root, "docs/generated/api.md", "# API\n");
    write(root, "pyproject.toml", "[project]\nname = 'example'\n");
    assert_eq!(run(root, &["init"], None).status.code(), Some(0));
    let config = std::fs::read_to_string(root.join("seiso.toml")).unwrap();
    assert!(config.contains("kind = \"reference\""));
    assert!(config.contains("kind = \"readme\""));
    assert!(!config.contains("kind = \"generated\""));
    assert_eq!(run(root, &["init"], None).status.code(), Some(2));
    assert_eq!(
        std::fs::read_to_string(root.join("seiso.toml")).unwrap(),
        config
    );
    let workspace = TempDir::new().unwrap();
    write(
        workspace.path(),
        "pyproject.toml",
        "[tool.seiso]\npreview = true\n",
    );
    assert_eq!(
        run(workspace.path(), &["init"], None).status.code(),
        Some(2)
    );
    assert!(!workspace.path().join("seiso.toml").exists());
}

#[test]
fn init_writes_at_the_repository_root_with_exclusions_and_community_kinds() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    write(root, "docs/guides/setup.md", "# Setup\n");
    write(root, ".github/ISSUE_TEMPLATE/bug.md", "# Bug\n");
    write(root, ".github/pull_request_template.md", "# Changes\n");
    write(root, ".github/CONTRIBUTING.md", "# Contributing\n");
    write(root, "SECURITY.md", "# Security\n");
    write(root, "CODE_OF_CONDUCT.md", "# Conduct\n");
    assert_eq!(
        run(&root.join("docs"), &["init"], None).status.code(),
        Some(0)
    );
    assert!(!root.join("docs/seiso.toml").exists());
    let config = std::fs::read_to_string(root.join("seiso.toml")).unwrap();
    for expected in [
        "\".github/ISSUE_TEMPLATE/**\"",
        "\".github/pull_request_template.md\"",
        "\"CODE_OF_CONDUCT.md\"",
        "path = \"docs/guides/**\"\nkind = \"howto\"",
        "path = \".github/CONTRIBUTING.md\"\nkind = \"howto\"",
        "path = \"SECURITY.md\"\nkind = \"howto\"",
    ] {
        assert!(config.contains(expected), "{config}");
    }
    let check = run(root, &["check", "--output-format", "json"], None);
    assert_eq!(check.status.code(), Some(0), "{}", value(&check));
    assert_eq!(
        std::fs::read_to_string(root.join(".seiso_cache/.gitignore")).unwrap(),
        "# Automatically created by seiso.\n*\n"
    );
    assert!(root.join(".seiso_cache/CACHEDIR.TAG").is_file());
}

#[test]
fn explicit_configuration_keeps_the_repository_root_and_applies_patterns_from_it() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    write(
        root,
        "ci/seiso.toml",
        "[lint]\nselect = ['KND']\n\n[[kinds]]\npath = 'docs/**'\nkind = 'howto'\n",
    );
    write(root, "README.md", "# Intro\n");
    write(root, "docs/setup.md", "# Setup\n");
    let output = run(
        root,
        &[
            "check",
            "--config",
            "ci/seiso.toml",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(diagnostics[0]["filename"], "README.md");
    let nested = run(
        &root.join("docs"),
        &[
            "check",
            "--config",
            "../ci/seiso.toml",
            "setup.md",
            "../README.md",
            "--output-format",
            "json",
        ],
        None,
    );
    assert_eq!(nested.status.code(), Some(1));
    assert_eq!(value(&nested), diagnostics);
}

#[test]
fn unchecked_inputs_and_inactive_preview_selectors_explain_themselves() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "seiso.toml", "exclude = ['drafts/**']\n");
    write(root, ".gitignore", "vendor/\n");
    write(root, "notes.txt", "Text\n");
    write(root, "vendor/lib/README.md", "# Vendored\n");
    write(root, "drafts/idea.md", "# Idea\n");
    write(root, "empty/.keep", "");
    write(root, "a.md", "---\nkind: reference\n---\n# A\n");
    let output = run(
        root,
        &[
            "check",
            "notes.txt",
            "vendor/lib/README.md",
            "vendor",
            "drafts",
            "drafts/idea.md",
            "empty",
            "a.md",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "seiso: notes.txt: Not checked because it is not a .md or .markdown file.",
        "seiso: vendor/lib/README.md: Not checked because .gitignore ignores it.",
        "seiso: vendor: Not checked because .gitignore ignores it.",
        "seiso: drafts: Not checked because configuration excludes its 1 Markdown file;",
        "seiso: drafts/idea.md: Not checked because `exclude` in seiso.toml matches it.",
        "seiso: empty: Not checked because it contains no Markdown files.",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
    assert!(!stderr.contains("seiso: a.md:") && !stderr.contains("No rules enabled"));

    write(root, "seiso.toml", "include = ['docs/**']\n");
    let empty = run(root, &["check"], None);
    assert_eq!(empty.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&empty.stderr).contains("No documents were checked in workspace")
    );

    write(root, "seiso.toml", "");
    let selectors = [
        "check",
        "a.md",
        "--select",
        "VOX,KND001",
        "--extend-select",
        "STL001",
    ];
    let inactive = String::from_utf8_lossy(&run(root, &selectors, None).stderr).into_owned();
    assert!(
        inactive.contains("VOX selects only preview rules"),
        "{inactive}"
    );
    assert!(
        inactive.contains("STL001 selects only preview rules"),
        "{inactive}"
    );
    assert!(!inactive.contains("KND001 selects"), "{inactive}");
    let enabled = run(root, &[&selectors[..], &["--preview"]].concat(), None);
    assert!(!String::from_utf8_lossy(&enabled.stderr).contains("selects only preview rules"));
    write(root, "seiso.toml", "preview = true\n");
    let configured = run(root, &selectors, None);
    assert!(!String::from_utf8_lossy(&configured.stderr).contains("selects only preview rules"));
}

#[test]
fn unchecked_directories_name_git_ignored_markdown_as_the_cause() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, ".gitignore", "*.md\n");
    write(root, "docs/guide.md", "# Guide\n");
    write(root, "empty/notes.txt", "Text\n");
    let whole = run(root, &["check"], None);
    assert_eq!(whole.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&whole.stderr);
    assert!(
        stderr.contains("because .gitignore ignores its Markdown files."),
        "{stderr}"
    );
    let named = run(root, &["check", "docs", "empty"], None);
    let stderr = String::from_utf8_lossy(&named.stderr);
    for expected in [
        "seiso: docs: Not checked because .gitignore ignores its Markdown files.",
        "seiso: empty: Not checked because it contains no Markdown files.",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn rule_documents_are_available_and_future_features_are_rejected() {
    let workspace = workspace();
    let root = workspace.path();
    let output = run(root, &["rule", "KND001"], None);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("KND001"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Status: stable."));
    let preview = run(root, &["rule", "STL001"], None);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("Status: preview."));
    let all = run(root, &["rule", "--all"], None);
    assert_eq!(all.status.code(), Some(0));
    let all_text = String::from_utf8_lossy(&all.stdout);
    assert_eq!(all_text.matches("Status: stable.").count(), 5);
    assert_eq!(all_text.matches("Status: preview.").count(), 22);
    for code in [
        "KND001", "KND002", "STL001", "STL003", "PTR001", "PTR003", "LNK001", "RAT002", "VOX001",
        "SUP001", "SUP002", "LNK002", "PTR002", "DUP001", "DUP002", "DUP003", "OWN001", "OWN002",
        "STL002", "STL004", "RAT001", "ORD001", "ORD002", "MIX001", "VOX002", "VOX003", "EVD001",
    ] {
        assert!(String::from_utf8_lossy(&all.stdout).contains(code));
    }
    for arguments in [
        vec!["rule", "LNK999"],
        vec!["rule", "STL999"],
        vec!["check", "--select", "STL999"],
        vec!["check", "--preview", "--select", "STL999"],
        vec!["check", "--judge"],
        vec!["check", "--output-format", "yaml"],
    ] {
        assert_eq!(run(root, &arguments, None).status.code(), Some(2));
    }
}

#[test]
fn hook_uses_event_cwd_converts_exit_codes_and_keeps_stdout_empty() {
    let workspace = workspace();
    let root = workspace.path();
    write(root, "a.md", "# Missing kind\n");
    let elsewhere = TempDir::new().unwrap();
    let event = |path: &str| {
        json!({"hook_event_name": "PostToolUse", "cwd": root, "tool_name": "Write", "tool_input": {"file_path": path}}).to_string()
    };
    let output = run(
        elsewhere.path(),
        &["hook", "claude-code"],
        Some(&event("a.md")),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("KND001"));
    write(root, "valid.md", "---\nkind: reference\n---\n# Valid\n");
    std::fs::write(root.join("unreadable.md"), [0xff]).unwrap();
    let valid = run(root, &["hook", "claude-code"], Some(&event("valid.md")));
    assert_eq!(valid.status.code(), Some(0));
    assert!(valid.stdout.is_empty() && valid.stderr.is_empty());
    let other = run(root, &["hook", "claude-code"], Some(&event("code.rs")));
    assert_eq!(other.status.code(), Some(0));
    assert!(other.stdout.is_empty() && other.stderr.is_empty());
    let missing = run(root, &["hook", "claude-code"], Some(&event("missing.md")));
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("does not exist"));
    for input in [
        "not json",
        "{}",
        r#"{"cwd":"relative","tool_input":{"file_path":"a.md"}}"#,
    ] {
        let malformed = run(root, &["hook", "claude-code"], Some(input));
        assert_eq!(malformed.status.code(), Some(1));
        assert!(malformed.stdout.is_empty());
    }
}
