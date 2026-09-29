//! Regression contracts for configuration frames and additive inheritance.
//!
//! Fixtures live beneath this checkout's `.temp` directory, rather than in a
//! system temporary directory, so reproductions stay inside the project.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use seiso::config::{CliOverrides, Config, Workspace};
use seiso::workspace::{self, LoadOptions, LoadScope};
use tempfile::{Builder, TempDir};

/// Create an independent, project-local workspace for one protocol case.
fn fixture() -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".temp");
    fs::create_dir_all(&root).unwrap();
    Builder::new()
        .prefix("frame-test-")
        .tempdir_in(root)
        .unwrap()
}

/// Write a fixture file, creating only its parent directories.
fn write(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Execute the public CLI from a fixture directory.
fn run(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn governing_parent_patterns_keep_their_frame() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
preview = true
include = ["docs/**/*.md"]
exclude = ["docs/private/**"]
[[kinds]]
path = "docs/reference/**"
kind = "reference"
[[domains]]
path = "docs/reference/**"
name = "api"
[lint]
select = ["DUP"]
[lint.per-file-ignores]
"docs/reference/**" = ["DUP"]
"#,
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    let reference = root.join("docs/reference/page.md");
    assert!(
        config.includes(&reference),
        "parent include is relative to repository root"
    );
    assert!(config.excludes(&root.join("docs/private/secret.md")));
    assert_eq!(config.kind_for(&reference), Some("reference"));
    assert_eq!(config.domain_for(&reference), Some("api"));
    assert!(
        !config
            .selected_rules(&reference, &CliOverrides::default())
            .unwrap()
            .contains(&"DUP001"),
        "parent per-file ignore must keep its own frame"
    );
}

#[cfg(windows)]
#[test]
fn governing_parent_identity_respects_windows_filename_case_insensitivity() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[[kinds]]\npath = 'docs/reference/**'\nkind = 'reference'\n",
    );
    write(root, "docs/seiso.toml", "extend = '../SEISO.toml'\n");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert_eq!(
        config.kind_for(&root.join("docs/reference/page.md")),
        Some("reference"),
        "case-only spelling differences must not turn a governing parent into a shared template"
    );
    assert_eq!(
        config.kind_for(&root.join("docs/docs/reference/page.md")),
        None
    );
}

#[test]
fn each_frame_in_three_level_chain_has_a_distinct_binding() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "templates/base.toml",
        r#"
[[kinds]]
path = "docs/template/**"
kind = "howto"
"#,
    );
    write(
        root,
        "seiso.toml",
        r#"
extend = "templates/base.toml"
[[domains]]
path = "docs/**"
name = "parent"
"#,
    );
    write(
        root,
        "docs/seiso.toml",
        r#"
extend = "../seiso.toml"
[[sites]]
path = "local/**"
root = "."
"#,
    );
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    let template_file = root.join("docs/template/page.md");
    let local_file = root.join("docs/local/page.md");
    assert_eq!(
        config.kind_for(&template_file),
        Some("howto"),
        "shared template rebases at governing parent"
    );
    assert_eq!(
        config.domain_for(&template_file),
        Some("parent"),
        "governing parent keeps its root frame"
    );
    assert!(
        config.site_for(&local_file).is_some(),
        "child site uses child frame"
    );
    assert!(config.site_for(&root.join("local/page.md")).is_none());
}

#[test]
fn shared_template_recomputes_relation_when_it_extends_its_own_governing_parent() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[[kinds]]\npath = 'docs/parent/**'\nkind = 'reference'\n",
    );
    write(
        root,
        "policies/template.toml",
        "extend = '../seiso.toml'\n[[extend-kinds]]\npath = 'local/**'\nkind = 'howto'\n",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../policies/template.toml'\n",
    );
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert_eq!(
        config.kind_for(&root.join("docs/parent/page.md")),
        Some("reference"),
        "template's own governing ancestor must bind to repository root"
    );
    assert_eq!(
        config.kind_for(&root.join("docs/local/page.md")),
        Some("howto"),
        "shared template's declarations must bind to importing child"
    );
    assert_eq!(config.kind_for(&root.join("policies/local/page.md")), None);
    assert_eq!(
        config.kind_for(&root.join("docs/docs/parent/page.md")),
        None
    );
}

#[test]
fn descendant_shared_template_uses_importing_child_frame() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "docs/sub/template.toml",
        "[[kinds]]\npath = 'local/**'\nkind = 'runbook'\n",
    );
    write(root, "docs/seiso.toml", "extend = 'sub/template.toml'\n");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert_eq!(
        config.kind_for(&root.join("docs/local/page.md")),
        Some("runbook")
    );
    assert_eq!(config.kind_for(&root.join("docs/sub/local/page.md")), None);
}

#[test]
fn explicit_config_keeps_workspace_rebasing_contract() {
    let dir = fixture();
    let root = dir.path();
    write(root, ".git", "gitdir: unused");
    write(
        root,
        "policies/base.toml",
        "[[kinds]]\npath = 'docs/**'\nkind = 'howto'\n",
    );
    write(root, "policies/selected.toml", "extend = 'base.toml'\n");
    let workspace = Workspace::discover(root, Some(Path::new("policies/selected.toml"))).unwrap();
    assert_eq!(workspace.root, root);
    let config = workspace.config_for(Path::new("docs/page.md")).unwrap();
    assert_eq!(config.kind_for(&root.join("docs/page.md")), Some("howto"));
    assert_eq!(config.kind_for(&root.join("policies/docs/page.md")), None);
}

#[test]
fn non_governing_ancestor_file_is_a_shared_template() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[[kinds]]\npath = 'local/**'\nkind = 'howto'\n",
    );
    write(root, "docs/.seiso.toml", "extend = '../seiso.toml'\n");
    // The same source is governing when selected by discovery.
    let governing = Config::load(&root.join("docs/.seiso.toml")).unwrap();
    assert_eq!(governing.kind_for(&root.join("docs/local/page.md")), None);
    assert_eq!(
        governing.kind_for(&root.join("local/page.md")),
        Some("howto")
    );

    fs::remove_file(root.join("docs/.seiso.toml")).unwrap();
    write(
        root,
        "base.toml",
        "[[kinds]]\npath = 'local/**'\nkind = 'howto'\n",
    );
    write(root, "docs/seiso.toml", "extend = '../base.toml'\n");
    let template = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert_eq!(
        template.kind_for(&root.join("docs/local/page.md")),
        Some("howto")
    );
    assert_eq!(template.kind_for(&root.join("local/page.md")), None);
}

#[test]
fn inherited_site_root_and_public_keep_parent_binding_in_fixed_workspace() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
preview = true
[lint]
select = ["LNK001", "LNK002"]
[[sites]]
path = "site/**"
root = "site"
public = "assets"
"#,
    );
    write(
        root,
        "site/guide/start.md",
        "# Start\n\n[page](/guide/target) [asset](/logo.png)\n",
    );
    write(root, "site/guide/target.md", "# Target\n");
    write(root, "assets/logo.png", "image");
    let baseline = run(
        root,
        &["check", "site/guide/start.md", "--output-format", "json"],
    );
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&baseline.stdout).unwrap(),
        serde_json::json!([])
    );

    write(
        root,
        "site/guide/seiso.toml",
        "extend = '../../seiso.toml'\n",
    );
    let config = Config::load(&root.join("site/guide/seiso.toml")).unwrap();
    assert!(config.site_for(&root.join("site/guide/start.md")).is_some());
    let inherited = run(
        root,
        &["check", "site/guide/start.md", "--output-format", "json"],
    );
    assert_eq!(
        inherited.status.code(),
        baseline.status.code(),
        "{}",
        String::from_utf8_lossy(&inherited.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&inherited.stdout).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&baseline.stdout).unwrap(),
        "site page and public routes must survive an empty inheriting child"
    );
}

#[test]
fn child_invocation_has_narrow_selection_but_parent_site_routes_are_project_in_scope() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[lint]\nselect = ['LNK001']\n[[sites]]\npath = 'docs/**'\nroot = '.'\npublic = 'public'\n",
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    write(
        root,
        "docs/a.md",
        "# Child\n\n[sibling](/sibling/page) [logo](/logo.png) [local](/local.md)\n",
    );
    write(root, "docs/local.md", "# Local\n");
    write(root, "sibling/page.md", "# Sibling\n");
    write(root, "public/logo.png", "image");
    // This would report LNK001 if default selection accidentally widened.
    write(root, "README.md", "# Root\n\n[missing](/absent)\n");
    let child = &root.join("docs");
    let policy = run(child, &["policy"]);
    assert!(
        policy.status.success(),
        "{}",
        String::from_utf8_lossy(&policy.stderr)
    );
    let policy: serde_json::Value = serde_json::from_slice(&policy.stdout).unwrap();
    assert!(policy["errors"].as_array().unwrap().is_empty());
    let names: Vec<_> = policy["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["filename"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["a.md", "local.md"],
        "default presentation and selection must remain child-relative"
    );
    assert!(policy["files"][0]["site"].is_object());

    let check = run(child, &["check", "--output-format", "json"]);
    assert!(
        check.status.success(),
        "status={:?} stdout={} stderr={}",
        check.status.code(),
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(
        diagnostics,
        serde_json::json!([]),
        "sibling page, root public asset, and child-root written link must resolve"
    );
}

#[test]
fn child_lnk002_uses_sibling_anchor_dependency_without_selecting_parent_files() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "preview = true\n[lint]\nselect = ['LNK002']\n[[sites]]\npath = 'docs/**'\nroot = '.'\n",
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    write(
        root,
        "docs/a.md",
        "# Child\n\n[valid](/sibling/page#present) [invalid](/sibling/page#absent)\n",
    );
    write(root, "sibling/page.md", "# Sibling\n\n## Present\n");
    write(
        root,
        "README.md",
        "# Root\n\n[unrelated bad anchor](sibling/page.md#missing)\n",
    );
    let check = run(&root.join("docs"), &["check", "--output-format", "json"]);
    assert_eq!(
        check.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let diagnostics = diagnostics.as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0]["code"], "LNK002");
    assert_eq!(diagnostics[0]["filename"], "a.md");
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("absent")
    );
}

/// The invocation policy is standalone; only the nested file explicitly imports root scope.
fn nested_project_scope_fixture() -> TempDir {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[lint]\nselect = ['LNK001']\n[[sites]]\npath = 'docs/sub/**'\nroot = '.'\npublic = 'assets'\n",
    );
    write(root, "README.md", "# Root\n");
    write(root, "assets/logo.png", "image");
    write(root, "docs/seiso.toml", "[lint]\nselect = ['LNK001']\n");
    write(root, "docs/sub/seiso.toml", "extend = '../../seiso.toml'\n");
    write(root, "docs/sub/a.md", "# Nested\n\n[logo](/logo.png)\n");
    write(
        root,
        "docs/neighbor.md",
        "# Neighbor\n\n[parent](../README.md)\n",
    );
    dir
}

#[test]
fn nested_explicit_parent_extends_site_resolution_beyond_invocation_policy() {
    let dir = nested_project_scope_fixture();
    let check = run(
        &dir.path().join("docs"),
        &["check", "sub/a.md", "--output-format", "json"],
    );
    assert!(
        check.status.success(),
        "status={:?} stdout={} stderr={}",
        check.status.code(),
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(diagnostics, serde_json::json!([]));
}

#[test]
fn explicit_nested_importer_admits_parent_path_only_in_the_same_request() {
    let dir = nested_project_scope_fixture();
    let child = dir.path().join("docs");
    let admitted = run(
        &child,
        &[
            "check",
            "sub/a.md",
            "../README.md",
            "--output-format",
            "json",
        ],
    );
    assert!(
        admitted.status.success(),
        "status={:?} stdout={} stderr={}",
        admitted.status.code(),
        String::from_utf8_lossy(&admitted.stdout),
        String::from_utf8_lossy(&admitted.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&admitted.stdout).unwrap();
    assert_eq!(diagnostics, serde_json::json!([]));

    let parsed = run(
        &child,
        &[
            "parse",
            "sub/a.md",
            "../README.md",
            "--output-format",
            "json",
        ],
    );
    assert!(
        parsed.status.success(),
        "status={:?} stdout={} stderr={}",
        parsed.status.code(),
        String::from_utf8_lossy(&parsed.stdout),
        String::from_utf8_lossy(&parsed.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&parsed.stdout).unwrap();
    let names: Vec<_> = parsed["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["filename"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["../README.md", "sub/a.md"],
        "explicit parent and child names remain invocation-relative"
    );

    let missing = run(
        &child,
        &["parse", "sub/a.md", "missing.md", "--output-format", "json"],
    );
    assert_eq!(missing.status.code(), Some(2));
    let missing: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing["errors"][0]["filename"], "missing.md");

    let unadmitted = run(
        &child,
        &["check", "../README.md", "--output-format", "json"],
    );
    assert_eq!(
        unadmitted.status.code(),
        Some(2),
        "parent path must remain outside scope without a selected importer; stdout={} stderr={}",
        String::from_utf8_lossy(&unadmitted.stdout),
        String::from_utf8_lossy(&unadmitted.stderr)
    );
}

#[test]
fn nested_parent_scope_does_not_admit_parent_link_for_standalone_neighbor() {
    let dir = nested_project_scope_fixture();
    let dump = run(&dir.path().join("docs"), &["index", "--dump"]);
    assert!(
        dump.status.success(),
        "{}",
        String::from_utf8_lossy(&dump.stderr)
    );
    let dump: serde_json::Value = serde_json::from_slice(&dump.stdout).unwrap();
    let neighbor = dump["index"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["filename"] == "neighbor.md")
        .unwrap();
    assert_eq!(
        neighbor["links"][0]["resolution"]["status"], "outside_workspace",
        "a nested config's root inheritance must not silently widen a standalone sibling's authority"
    );
}

#[test]
fn nested_project_dependency_does_not_pair_standalone_neighbor_with_root_duplicate() {
    const DEFINITIONS: &str = "- `host`: Server address.\n- `port`: Listening port.\n- `user`: Account name.\n- `token`: Access token.\n- `timeout`: Request timeout.\n";
    let dir = fixture();
    let root = dir.path();
    let lint = "preview = true\n[lint]\nselect = ['DUP001', 'OWN001']\n";
    write(root, ".git", "gitdir: unused");
    write(root, "seiso.toml", lint);
    write(
        root,
        "reference.md",
        &format!("---\nkind: reference\nlang: en\n---\n\n# Settings\n\n{DEFINITIONS}"),
    );
    write(root, "docs/seiso.toml", lint);
    write(
        root,
        "docs/neighbor.md",
        &format!("---\nkind: plan\nlang: en\n---\n\n# Settings\n\n{DEFINITIONS}"),
    );
    write(root, "docs/sub/seiso.toml", "extend = '../../seiso.toml'\n");
    write(root, "docs/sub/nested.md", "# Nested\n");

    // Sanity control: when root and neighbor deliberately share one project,
    // these exact documents produce the expected duplication edge.
    let control = run(
        root,
        &[
            "check",
            "docs/neighbor.md",
            "--config",
            "seiso.toml",
            "--output-format",
            "json",
        ],
    );
    assert_eq!(
        control.status.code(),
        Some(1),
        "control stdout={} stderr={}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let control: serde_json::Value = serde_json::from_slice(&control.stdout).unwrap();
    assert!(
        control
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "DUP001" && item["filename"] == "docs/neighbor.md"),
        "fixture must actually contain a detectable duplicate: {control}"
    );

    let snapshot = workspace::load(
        &root.join("docs"),
        &LoadOptions::default(),
        LoadScope::Check,
    )
    .unwrap();
    assert_eq!(
        snapshot.index.root, root,
        "nested inheritance must actually widen the dependency index in this test"
    );
    assert!(
        snapshot.index.file("reference.md").is_some(),
        "root duplicate must be indexed; otherwise isolation passes vacuously"
    );

    // The nested file can import the ancestor project, but the standalone
    // neighbor must retain the child invocation's narrower comparison scope.
    let isolated = run(&root.join("docs"), &["check", "--output-format", "json"]);
    assert!(
        isolated.status.success(),
        "isolated stdout={} stderr={}",
        String::from_utf8_lossy(&isolated.stdout),
        String::from_utf8_lossy(&isolated.stderr)
    );
    let isolated: serde_json::Value = serde_json::from_slice(&isolated.stdout).unwrap();
    assert_eq!(isolated, serde_json::json!([]));
}

#[test]
fn root_invocation_keeps_cross_file_and_link_scope_across_standalone_child_config() {
    const DEFINITIONS: &str = "- `host`: Server address.\n- `port`: Listening port.\n- `user`: Account name.\n- `token`: Access token.\n- `timeout`: Request timeout.\n";
    let dir = fixture();
    let root = dir.path();
    let lint = "preview = true\n[lint]\nselect = ['LNK001', 'DUP001']\n";
    write(root, ".git", "gitdir: unused");
    write(root, "seiso.toml", lint);
    write(root, "README.md", "# Root\n");
    write(
        root,
        "reference.md",
        &format!("---\nkind: reference\nlang: en\n---\n\n# Settings\n\n{DEFINITIONS}"),
    );
    write(root, "docs/seiso.toml", lint);
    write(
        root,
        "docs/neighbor.md",
        &format!(
            "---\nkind: plan\nlang: en\n---\n\n# Settings\n\n{DEFINITIONS}\n[root](../README.md)\n"
        ),
    );
    let check = run(
        root,
        &["check", "docs/neighbor.md", "--output-format", "json"],
    );
    assert_eq!(
        check.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let diagnostics = diagnostics.as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|item| item["code"] == "DUP001" && item["filename"] == "docs/neighbor.md"),
        "root invocation must retain the root-versus-docs comparison: {diagnostics:?}"
    );
    assert!(
        diagnostics.iter().all(|item| item["code"] != "LNK001"),
        "a standalone child config must not block a root-invocation link to README: {diagnostics:?}"
    );
    let dump = run(root, &["index", "--dump"]);
    assert!(
        dump.status.success(),
        "{}",
        String::from_utf8_lossy(&dump.stderr)
    );
    let dump: serde_json::Value = serde_json::from_slice(&dump.stdout).unwrap();
    let neighbor = dump["index"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["filename"] == "docs/neighbor.md")
        .unwrap();
    assert_eq!(neighbor["links"][0]["resolution"]["status"], "file");
    assert_eq!(neighbor["links"][0]["resolution"]["target"], "README.md");
}

#[test]
fn explicit_admission_closure_is_independent_of_argument_order() {
    let dir = fixture();
    let outer = dir.path();
    write(outer, "seiso.toml", "");
    write(outer, "OUTER.md", "# Outer\n");
    write(outer, "repo/seiso.toml", "");
    write(outer, "repo/docs/seiso.toml", "");
    write(
        outer,
        "repo/docs/sub/seiso.toml",
        "extend = '../../seiso.toml'\n",
    );
    write(outer, "repo/docs/sub/a.md", "# Child\n");
    write(
        outer,
        "repo/peer/seiso.toml",
        "extend = '../../seiso.toml'\n",
    );
    write(outer, "repo/peer/a.md", "# Peer\n");
    let docs = outer.join("repo/docs");
    let arguments = ["sub/a.md", "../peer/a.md", "../../OUTER.md"];
    for order in [arguments, [arguments[2], arguments[1], arguments[0]]] {
        let parsed = run(
            &docs,
            &[
                "parse",
                order[0],
                order[1],
                order[2],
                "--output-format",
                "json",
            ],
        );
        assert!(
            parsed.status.success(),
            "order={order:?} stderr={} stdout={}",
            String::from_utf8_lossy(&parsed.stderr),
            String::from_utf8_lossy(&parsed.stdout)
        );
        let report: serde_json::Value = serde_json::from_slice(&parsed.stdout).unwrap();
        let filenames: Vec<_> = report["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["filename"].as_str().unwrap())
            .collect();
        assert_eq!(filenames, ["../../OUTER.md", "sub/a.md", "../peer/a.md"]);
    }
}

#[cfg(unix)]
#[test]
fn explicit_alias_into_parent_waits_for_selected_frame_admission() {
    use std::os::unix::fs::symlink;

    let dir = nested_project_scope_fixture();
    let docs = dir.path().join("docs");
    symlink("../README.md", docs.join("alias.md")).unwrap();
    let admitted = run(
        &docs,
        &["parse", "sub/a.md", "alias.md", "--output-format", "json"],
    );
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&admitted.stdout).unwrap();
    let names: Vec<_> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["filename"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["../README.md", "sub/a.md"]);
    let denied = run(&docs, &["parse", "alias.md", "--output-format", "json"]);
    assert_eq!(denied.status.code(), Some(2));
}

#[test]
fn declared_project_allows_explicit_parent_file_but_keeps_sibling_policy() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[lint]\nselect = ['LNK001']\n[[kinds]]\npath = 'sibling/**'\nkind = 'howto'\n",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../seiso.toml'\n[[extend-kinds]]\npath = 'local.md'\nkind = 'adr'\n",
    );
    write(root, "docs/local.md", "# Local\n");
    write(root, "README.md", "# Root\n\n[missing](/absent)\n");
    write(
        root,
        "sibling/seiso.toml",
        "[[kinds]]\npath = '**'\nkind = 'runbook'\n",
    );
    write(root, "sibling/page.md", "# Sibling\n");
    let child = root.join("docs");
    let workspace = Workspace::discover(&child, None).unwrap();
    let sibling = workspace.config_for(&root.join("sibling/page.md")).unwrap();
    assert_eq!(
        sibling.kind_for(&root.join("sibling/page.md")),
        Some("runbook")
    );
    let parent = workspace.config_for(&root.join("README.md")).unwrap();
    assert_eq!(
        parent.source.as_deref(),
        Some(root.join("seiso.toml").as_path())
    );

    let parsed = run(
        &child,
        &["parse", "../sibling/page.md", "--output-format", "json"],
    );
    assert!(
        parsed.status.success(),
        "{}",
        String::from_utf8_lossy(&parsed.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&parsed.stdout).unwrap();
    assert_eq!(parsed["files"][0]["filename"], "../sibling/page.md");
    assert_eq!(parsed["files"][0]["kind"]["value"], "runbook");
    assert_eq!(parsed["files"][0]["kind"]["source"], "configuration");

    let selected = run(
        &child,
        &["check", "../README.md", "--output-format", "json"],
    );
    assert_eq!(
        selected.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(diagnostics.as_array().unwrap().len(), 1, "{diagnostics}");
    assert_eq!(diagnostics[0]["filename"], "../README.md");
    assert_eq!(diagnostics[0]["code"], "LNK001");
}

#[test]
fn non_governing_external_template_does_not_admit_its_directory_to_project() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "template.toml",
        "[[kinds]]\npath = 'local.md'\nkind = 'howto'\n",
    );
    write(root, "outside.md", "# Outside\n");
    write(root, "repo/seiso.toml", "extend = '../template.toml'\n");
    write(root, "repo/local.md", "# Local\n");
    let workspace = Workspace::discover(&root.join("repo"), None).unwrap();
    assert_eq!(
        workspace.config.kind_for(&root.join("repo/local.md")),
        Some("howto")
    );
    assert!(
        workspace.config_for(&root.join("outside.md")).is_err(),
        "external template is policy, not a project-scope declaration"
    );
    let outside = run(
        &root.join("repo"),
        &["check", "../outside.md", "--output-format", "json"],
    );
    assert_eq!(
        outside.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&outside.stderr)
    );
}

/// Create a file symlink when the test account has the necessary capability.
#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows test runners may lack Developer Mode or symlink privilege.
#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[test]
fn parent_public_directory_does_not_expose_a_symlink_outside_declared_project() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[lint]\nselect = ['LNK001']\n[[sites]]\npath = 'docs/**'\nroot = '.'\npublic = 'public'\n",
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    write(root, "docs/a.md", "# Child\n\n[escape](/escape.png)\n");
    let exterior = fixture();
    write(exterior.path(), "escape.png", "outside secret");
    fs::create_dir_all(root.join("public")).unwrap();
    if let Err(error) = symlink_file(
        &exterior.path().join("escape.png"),
        &root.join("public/escape.png"),
    ) {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("Skipping symlink case: symlink privilege unavailable: {error}");
            return;
        }
        panic!("Cannot create symlink fixture: {error}");
    }
    let dump = run(&root.join("docs"), &["index", "--dump"]);
    assert!(
        dump.status.success(),
        "{}",
        String::from_utf8_lossy(&dump.stderr)
    );
    let dump: serde_json::Value = serde_json::from_slice(&dump.stdout).unwrap();
    let files = dump["index"]["files"].as_array().unwrap();
    let page = files
        .iter()
        .find(|file| {
            file["filename"]
                .as_str()
                .is_some_and(|name| name.ends_with("a.md"))
        })
        .unwrap();
    assert_eq!(
        page["links"][0]["resolution"]["status"], "outside_workspace",
        "symlink target must not be opened as a project file"
    );
}

#[test]
fn inherited_pointer_catalog_directory_keeps_its_frame() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[lint.ptr]\ncatalog-dirs = ['docs/catalog/']\n",
    );
    write(root, "docs/seiso.toml", "extend = '../seiso.toml'\n");
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert!(config.is_catalog_dir(&root.join("docs/catalog")));
    assert!(!config.is_catalog_dir(&root.join("docs/docs/catalog")));
}

#[test]
fn per_file_ignore_key_replacement_updates_value_and_frame_together() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
preview = true
[lint]
select = ['DUP001', 'KND001']
[lint.per-file-ignores]
'docs/**' = ['DUP001']
"#,
    );
    write(
        root,
        "docs/seiso.toml",
        r#"
extend = '../seiso.toml'
[lint.per-file-ignores]
'docs/**' = ['KND001']
'local/**' = ['DUP001']
"#,
    );
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    let selected = |path: &str| {
        config
            .selected_rules(&root.join(path), &CliOverrides::default())
            .unwrap()
    };
    let parent_frame = selected("docs/other/page.md");
    assert!(
        parent_frame.contains(&"DUP001"),
        "overridden parent selector must not survive: {parent_frame:?}"
    );
    assert!(
        parent_frame.contains(&"KND001"),
        "child's same-text key is child-bound, not parent-bound"
    );
    let child_frame = selected("docs/local/page.md");
    assert!(
        !child_frame.contains(&"DUP001"),
        "child-local ignore must bind to child directory"
    );
    assert!(child_frame.contains(&"KND001"));
}

#[test]
fn additive_mappings_accumulate_after_effective_base_in_chain_order() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        r#"
[[kinds]]
path = "docs/**"
kind = "howto"
[[extend-kinds]]
path = "docs/special/**"
kind = "reference"
[[domains]]
path = "docs/**"
name = "root"
[[extend-domains]]
path = "docs/special/**"
name = "special"
[[sites]]
path = "docs/**"
root = "docs"
base = "/root/"
[[extend-sites]]
path = "docs/special/**"
root = "docs/special"
base = "/special/"
"#,
    );
    write(
        root,
        "docs/seiso.toml",
        r#"
extend = "../seiso.toml"
[[extend-kinds]]
path = "special/page.md"
kind = "adr"
[[extend-domains]]
path = "special/page.md"
name = "page"
[[extend-sites]]
path = "special/page.md"
root = "special"
base = "/page/"
"#,
    );
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    let ordinary = root.join("docs/ordinary.md");
    let special = root.join("docs/special/other.md");
    let page = root.join("docs/special/page.md");
    assert_eq!(config.kind_for(&ordinary), Some("howto"));
    assert_eq!(config.kind_for(&special), Some("reference"));
    assert_eq!(
        config.kind_for(&page),
        Some("adr"),
        "later child extension wins"
    );
    assert_eq!(config.domain_for(&page), Some("page"));
    assert_eq!(config.site_for(&page).unwrap().base, "/page/");
    assert_eq!(config.settings.kinds.len(), 3);
    assert_eq!(config.settings.domains.len(), 3);
    assert_eq!(config.settings.sites.len(), 3);
}

#[test]
fn replacing_plain_mappings_does_not_discard_inherited_extensions() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "[[kinds]]\npath = 'docs/**'\nkind = 'howto'\n[[extend-kinds]]\npath = 'docs/special/**'\nkind = 'reference'\n",
    );
    write(
        root,
        "docs/seiso.toml",
        "extend = '../seiso.toml'\n[[kinds]]\npath = 'local/**'\nkind = 'runbook'\n",
    );
    let config = Config::load(&root.join("docs/seiso.toml")).unwrap();
    assert_eq!(
        config.kind_for(&root.join("docs/local/page.md")),
        Some("runbook")
    );
    assert_eq!(
        config.kind_for(&root.join("docs/special/page.md")),
        Some("reference")
    );
    assert_eq!(config.kind_for(&root.join("docs/ordinary.md")), None);
    assert_eq!(config.settings.kinds.len(), 2);
}

#[test]
fn malformed_additive_mapping_is_rejected_at_load_time() {
    for (name, entry) in [
        ("missing-kind", "[[extend-kinds]]\npath = 'docs/**'\n"),
        (
            "unknown-kind",
            "[[extend-kinds]]\npath = 'docs/**'\nkind = 'not-a-kind'\n",
        ),
        ("missing-site-root", "[[extend-sites]]\npath = 'docs/**'\n"),
        (
            "missing-domain-name",
            "[[extend-domains]]\npath = 'docs/**'\n",
        ),
    ] {
        let dir = fixture();
        write(dir.path(), "seiso.toml", "preview = true\n");
        write(
            dir.path(),
            "docs/seiso.toml",
            &format!("extend = '../seiso.toml'\n{entry}"),
        );
        assert!(
            Config::load(&dir.path().join("docs/seiso.toml")).is_err(),
            "{name}"
        );
    }
}

#[test]
fn init_extend_never_overwrites_an_existing_child_config() {
    let dir = fixture();
    let root = dir.path();
    write(root, "seiso.toml", "preview = true\n");
    write(root, "docs/seiso.toml", "preview = false\n");
    let existing = fs::read(root.join("docs/seiso.toml")).unwrap();
    let output = run(&root.join("docs"), &["init", "--extend"]);
    assert!(!output.status.success(), "existing config must be rejected");
    assert_eq!(fs::read(root.join("docs/seiso.toml")).unwrap(), existing);
}

#[test]
fn init_extend_creates_child_without_changing_existing_file_policy() {
    let dir = fixture();
    let root = dir.path();
    write(
        root,
        "seiso.toml",
        "include = ['docs/**/*.md']\n[[kinds]]\npath = 'docs/**'\nkind = 'howto'\n[[domains]]\npath = 'docs/**'\nname = 'all-docs'\n",
    );
    write(root, "docs/page.md", "# Page\n");
    let baseline = run(root, &["policy"]);
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
    let created = run(&root.join("docs"), &["init", "--extend"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let source = fs::read_to_string(root.join("docs/seiso.toml")).unwrap();
    assert!(source.contains("extend = \"../seiso.toml\""), "{source}");
    let after = run(root, &["policy"]);
    assert!(
        after.status.success(),
        "{}",
        String::from_utf8_lossy(&after.stderr)
    );
    let after: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    let file = |report: &serde_json::Value| {
        report["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["filename"] == "docs/page.md")
            .unwrap()
            .clone()
    };
    let before = file(&baseline);
    let after = file(&after);
    for field in ["kind", "domain", "site", "enabled_rules", "excluded"] {
        assert_eq!(
            after[field], before[field],
            "init --extend changed {field} for an existing file"
        );
    }
}
