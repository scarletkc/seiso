use common::write;
mod common;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn parse(root: &Path, arguments: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(root)
        .args(["parse", "--output-format", "json"])
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
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

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn configured_workspace() -> TempDir {
    let workspace = TempDir::new().unwrap();
    write(
        workspace.path(),
        "seiso.toml",
        "[[kinds]]\npath = '**/*.md'\nkind = 'reference'\n",
    );
    workspace
}

#[test]
fn repeated_runs_and_argument_order_produce_identical_bytes() {
    let workspace = configured_workspace();
    let root = workspace.path();
    write(root, "b.md", "# 日本語\n\n現在の設定を参照。\n");
    write(root, "a.md", "# 配置\n\nRead `src/config.rs`。\n");
    let first = parse(root, &[], None);
    let second = parse(root, &[], None);
    let selected = parse(root, &["b.md", "a.md", "a.md"], None);
    assert_eq!(first.status.code(), Some(0));
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stdout, selected.stdout);
    let report = json(&first);
    assert_eq!(report["files"][0]["filename"], "a.md");
    assert_eq!(report["files"][1]["filename"], "b.md");
    assert_eq!(
        report["files"][0]["section_annotations"][0]["section_type"],
        "other"
    );
}

#[test]
fn subdirectory_invocation_keeps_workspace_relative_paths() {
    let workspace = configured_workspace();
    write(workspace.path(), "docs/a.md", "# Heading\n");
    let full = parse(workspace.path(), &[], None);
    let nested = parse(&workspace.path().join("docs"), &[], None);
    assert_eq!(full.status.code(), Some(0));
    assert_eq!(full.stdout, nested.stdout);
}

#[test]
fn nested_config_replaces_parent_and_frontmatter_precedes_mapping() {
    let workspace = configured_workspace();
    let root = workspace.path();
    write(
        root,
        "seiso.toml",
        "exclude = ['docs/**']\n[[kinds]]\npath = '**/*.md'\nkind = 'reference'\n",
    );
    write(
        root,
        "docs/.seiso.toml",
        "[[kinds]]\npath = '*.md'\nkind = 'howto'\n",
    );
    write(root, "docs/a.md", "---\nkind: adr\n---\n# Decision\n");
    write(root, "docs/b.md", "# Steps\n");
    let output = parse(root, &[], None);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert_eq!(report["files"].as_array().unwrap().len(), 2);
    assert_eq!(report["files"][0]["kind"]["value"], "adr");
    assert_eq!(report["files"][1]["kind"]["value"], "howto");
    assert_eq!(report["files"][0]["configuration"], "docs/.seiso.toml");
}

#[test]
fn generated_cannot_be_claimed_by_a_document() {
    let workspace = configured_workspace();
    write(
        workspace.path(),
        "a.md",
        "---\nkind: generated\n---\n# Generated\n",
    );
    let output = parse(workspace.path(), &[], None);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert!(report["files"][0]["kind"]["value"].is_null());
    assert!(
        report["files"][0]["kind"]["problem"]
            .as_str()
            .unwrap()
            .contains("configuration")
    );
}

#[test]
fn malformed_frontmatter_does_not_fall_back_to_path_kind() {
    let workspace = configured_workspace();
    write(
        workspace.path(),
        "a.md",
        "---\nkind: [invalid\n---\n# Heading\n",
    );
    let output = parse(workspace.path(), &[], None);
    let report = json(&output);
    assert!(report["files"][0]["kind"]["value"].is_null());
    assert_eq!(report["files"][0]["kind"]["source"], "frontmatter");
    assert!(
        !report["files"][0]["document"]["frontmatter"]["errors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stdin_overlays_a_missing_or_existing_file_without_writing() {
    let workspace = configured_workspace();
    write(workspace.path(), "a.md", "# Disk\n");
    let output = parse(
        workspace.path(),
        &["--stdin-filename", "a.md"],
        Some("# Buffer\n"),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        json(&output)["files"][0]["document"]["source"],
        "# Buffer\n"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("a.md")).unwrap(),
        "# Disk\n"
    );
    let output = parse(
        workspace.path(),
        &["--stdin-filename", "new.md"],
        Some("# New\n"),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(json(&output)["files"][0]["filename"], "new.md");
    assert!(!workspace.path().join("new.md").exists());
}

#[test]
fn gitignore_applies_to_disk_and_stdin() {
    let workspace = configured_workspace();
    write(workspace.path(), ".gitignore", "ignored.md\n");
    write(workspace.path(), "ignored.md", "# Ignore me\n");
    write(workspace.path(), "included.md", "# Keep me\n");
    let output = parse(workspace.path(), &[], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(json(&output)["files"].as_array().unwrap().len(), 1);
    let output = parse(
        workspace.path(),
        &["--stdin-filename", "ignored.md"],
        Some("# Buffer\n"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains(".gitignore"));
}

#[test]
fn nested_gitignore_paths_and_parent_exclusions_match_disk_semantics() {
    let workspace = configured_workspace();
    let root = workspace.path();
    write(root, ".gitignore", "*.md\nblocked/\n");
    write(root, "docs/.gitignore", "!*.md\n/private.md\n");
    write(root, "blocked/.gitignore", "!allowed.md\n");
    write(root, "docs/private.md", "# Private\n");
    write(root, "docs/public.md", "# Public\n");
    write(root, "blocked/allowed.md", "# Blocked\n");
    let disk = parse(root, &[], None);
    assert_eq!(disk.status.code(), Some(0));
    assert_eq!(json(&disk)["files"].as_array().unwrap().len(), 1);
    assert_eq!(json(&disk)["files"][0]["filename"], "docs/public.md");
    for (path, code) in [
        ("docs/private.md", 2),
        ("docs/public.md", 0),
        ("blocked/allowed.md", 2),
    ] {
        let output = parse(root, &["--stdin-filename", path], Some("# Buffer\n"));
        assert_eq!(
            output.status.code(),
            Some(code),
            "{path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn unreadable_utf8_does_not_discard_other_documents() {
    let workspace = configured_workspace();
    write(workspace.path(), "a.md", "# Readable\n");
    std::fs::write(workspace.path().join("b.md"), [0xff, 0xfe, 0x00]).unwrap();
    let output = parse(workspace.path(), &[], None);
    assert_eq!(output.status.code(), Some(2));
    let report = json(&output);
    assert_eq!(report["files"].as_array().unwrap().len(), 1);
    assert_eq!(report["errors"][0]["filename"], "b.md");
    assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
}

#[test]
fn selected_parse_and_stdin_skip_unrelated_unreadable_sources_and_invalid_configs() {
    let workspace = configured_workspace();
    let root = workspace.path();
    write(root, "guide.md", "# Guide\n");
    write(root, "other/seiso.toml", "unexpected = true\n");
    write(root, "other/page.md", "# Other\n");
    std::fs::write(root.join("unreadable.md"), [0xff]).unwrap();
    let selected = parse(root, &["guide.md"], None);
    assert_eq!(selected.status.code(), Some(0));
    assert!(selected.stderr.is_empty());
    assert_eq!(json(&selected)["files"].as_array().unwrap().len(), 1);
    assert_eq!(json(&selected)["files"][0]["filename"], "guide.md");
    let stdin = parse(root, &["--stdin-filename", "buffer.md"], Some("# Buffer\n"));
    assert_eq!(stdin.status.code(), Some(0));
    assert!(stdin.stderr.is_empty());
    assert_eq!(json(&stdin)["files"].as_array().unwrap().len(), 1);
    assert_eq!(json(&stdin)["files"][0]["document"]["source"], "# Buffer\n");
    assert!(!root.join("buffer.md").exists());
}

#[test]
fn missing_requested_file_is_an_error_and_existing_file_still_parses() {
    let workspace = configured_workspace();
    write(workspace.path(), "a.md", "# Available\n");
    let output = parse(workspace.path(), &["absent.md", "a.md"], None);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["files"].as_array().unwrap().len(), 1);
    assert_eq!(json(&output)["errors"][0]["filename"], "absent.md");
}

#[test]
fn config_errors_return_two_without_decorating_json() {
    let workspace = configured_workspace();
    write(workspace.path(), "seiso.toml", "unexpected = true\n");
    let output = parse(workspace.path(), &[], None);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected"));
}

#[test]
fn invalid_gitignore_is_an_incomplete_parse_and_preserves_other_files() {
    let workspace = configured_workspace();
    write(workspace.path(), ".gitignore", "[z-a]\n");
    write(workspace.path(), "a.md", "# Heading\n");
    let output = parse(workspace.path(), &[], None);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["files"].as_array().unwrap().len(), 1);
    assert!(!json(&output)["errors"].as_array().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn existing_file_selection_uses_filesystem_case_semantics() {
    let workspace = configured_workspace();
    write(workspace.path(), "README.md", "# Heading\n");
    let exact = parse(workspace.path(), &["README.md"], None);
    let differently_cased = parse(workspace.path(), &["readme.md"], None);
    assert_eq!(exact.status.code(), Some(0));
    assert_eq!(exact.stdout, differently_cased.stdout);
}

#[test]
fn arguments_and_paths_cannot_silently_escape_the_workspace() {
    let workspace = configured_workspace();
    let output = parse(
        workspace.path(),
        &["--stdin-filename", "../outside.md"],
        Some("# Outside\n"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside workspace"));
    let output = parse(
        workspace.path(),
        &["--stdin-filename", "a.md", "b.md"],
        None,
    );
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn text_output_makes_the_inspection_boundary_explicit() {
    let workspace = configured_workspace();
    write(workspace.path(), "a.md", "# Heading\n\nParagraph.\n");
    let output = Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(workspace.path())
        .args(["parse", "a.md"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    insta::assert_snapshot!(String::from_utf8(output.stdout).unwrap());
}
