mod common;
use common::{run, value, workspace, write};

use std::process::Output;

use serde_json::json;
use tempfile::TempDir;

fn status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const LINKS: &str = "preview=true\n[lint]\nselect=['LNK001','LNK002','SUP002']\n";

#[test]
fn cli_diagnostics_include_versioned_rule_documentation_links() {
    let root = workspace("[lint]\nselect=['LNK001']\n");
    write(root.path(), "guide.md", "[Missing](missing.md)\n");
    let expected = format!(
        "https://github.com/scarletkc/seiso/blob/v{}/docs/rules/LNK001.md",
        env!("CARGO_PKG_VERSION")
    );
    let json = run(root.path(), &["check", "--output-format", "json"], None);
    status(&json, 1);
    assert!(json.stderr.is_empty());
    let json = value(&json);
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["url"], expected);
    let sarif = run(root.path(), &["check", "--output-format", "sarif"], None);
    status(&sarif, 1);
    assert!(sarif.stderr.is_empty());
    let sarif = value(&sarif);
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["rules"][0]["helpUri"],
        expected
    );
}

#[test]
fn cold_warm_and_disabled_caches_produce_identical_reports() {
    let root = workspace(LINKS);
    write(
        root.path(),
        "guide.md",
        "# Guide\n\n[Missing heading](reference.md#old)\n",
    );
    write(root.path(), "reference.md", "# Reference\n");
    let args = ["check", "--output-format", "json"];
    let cold = run(root.path(), &args, None);
    status(&cold, 1);
    assert!(root.path().join(".seiso_cache").is_dir());
    let warm = run(root.path(), &args, None);
    let fresh = run(
        root.path(),
        &["check", "--no-cache", "--output-format", "json"],
        None,
    );
    assert_eq!(warm.stdout, cold.stdout);
    assert_eq!(fresh.stdout, cold.stdout);
    assert!(warm.stderr.is_empty() && fresh.stderr.is_empty());
    assert_eq!(value(&cold)[0]["code"], "LNK002");
}

#[test]
fn editing_only_an_anchor_target_reports_affected_origins_and_matches_full_filter() {
    let root = workspace(LINKS);
    write(root.path(), "reference.md", "# Original\n");
    write(
        root.path(),
        "guide.md",
        "[Reference](reference.md#original)\n",
    );
    write(root.path(), "unrelated.md", "[Other](missing.md)\n");
    status(
        &run(
            root.path(),
            &["check", "reference.md", "--output-format", "json"],
            None,
        ),
        0,
    );
    write(root.path(), "reference.md", "# Renamed\n");
    let selected = run(
        root.path(),
        &["check", "reference.md", "--output-format", "json"],
        None,
    );
    status(&selected, 1);
    let diagnostics = value(&selected);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(diagnostics[0]["code"], "LNK002");
    assert_eq!(diagnostics[0]["filename"], "guide.md");
    assert_eq!(diagnostics[0]["related"][0]["filename"], "reference.md");
    let full = value(&run(
        root.path(),
        &["check", "--output-format", "json"],
        None,
    ));
    let expected: Vec<_> = full
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            diagnostic["filename"] == "reference.md"
                || diagnostic["related"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|related| related["filename"] == "reference.md")
        })
        .cloned()
        .collect();
    assert_eq!(diagnostics, json!(expected));
    std::fs::remove_file(root.path().join("reference.md")).unwrap();
    let deleted = run(
        root.path(),
        &["check", "guide.md", "--output-format", "json"],
        None,
    );
    status(&deleted, 1);
    assert_eq!(value(&deleted)[0]["code"], "LNK001");
}

#[test]
fn selected_target_without_index_rules_still_reports_affected_origins() {
    let root = workspace(
        "preview=true\n[lint]\nselect=['LNK002']\n[lint.per-file-ignores]\n'reference.md'=['ALL']\n",
    );
    write(root.path(), "reference.md", "# Renamed\n");
    write(
        root.path(),
        "guide.md",
        "[Original](reference.md#original)\n",
    );
    let output = run(
        root.path(),
        &[
            "check",
            "reference.md",
            "--no-cache",
            "--output-format",
            "json",
        ],
        None,
    );
    status(&output, 1);
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(diagnostics[0]["filename"], "guide.md");
    assert_eq!(diagnostics[0]["code"], "LNK002");
    assert_eq!(diagnostics[0]["related"][0]["filename"], "reference.md");
}

#[test]
fn selecting_a_directory_reports_links_targeting_the_directory_itself() {
    let root = workspace("preview=true\n[lint]\nselect=['PTR002']\n");
    write(
        root.path(),
        "guide.md",
        "---\nkind: howto\n---\nSee [the catalog](catalog/).\n",
    );
    write(
        root.path(),
        "catalog/entry.md",
        "---\nkind: reference\n---\n# Entry\n",
    );
    let full = run(root.path(), &["check", "--output-format", "json"], None);
    status(&full, 1);
    let selected = run(
        root.path(),
        &["check", "catalog", "--output-format", "json"],
        None,
    );
    status(&selected, 1);
    assert_eq!(selected.stdout, full.stdout);
    assert_eq!(value(&selected)[0]["related"][0]["filename"], "catalog");
}

#[test]
fn generated_documents_supply_anchors_and_run_no_rules() {
    let root = workspace(
        "preview=true\n[[kinds]]\npath='generated.md'\nkind='generated'\n[lint]\nselect=['KND001','LNK001','LNK002']\n",
    );
    write(
        root.path(),
        "generated.md",
        "# API\n\n[Broken](missing.md)\n",
    );
    write(
        root.path(),
        "guide.md",
        "---\nkind: howto\n---\n[API](generated.md#api)\n",
    );
    let output = run(root.path(), &["check", "--output-format", "json"], None);
    status(&output, 0);
    assert_eq!(value(&output), json!([]));
    let policy = value(&run(root.path(), &["policy"], None));
    assert_eq!(policy["files"][0]["kind"]["value"], "generated");
    assert_eq!(policy["files"][0]["enabled_rules"], json!([]));
    let index = value(&run(root.path(), &["index", "--dump"], None));
    assert!(index["index"]["files"][0]["anchors"].get("api").is_some());
}

#[test]
fn excluded_target_anchors_remain_unknown_and_do_not_stale_suppressions() {
    let root = workspace(
        "preview=true\nexclude=['target.md']\n[lint]\nselect=['LNK001','LNK002','SUP002']\n",
    );
    write(root.path(), "target.md", "# Target\n");
    write(
        root.path(),
        "guide.md",
        "<!-- seiso: allow LNK002 -- Excluded external conventions. -->\n[Target](target.md#absent)\n",
    );
    let output = run(root.path(), &["check", "--output-format", "json"], None);
    status(&output, 0);
    assert_eq!(value(&output), json!([]));
    let index = value(&run(root.path(), &["index", "--dump"], None));
    assert_eq!(
        index["index"]["files"][0]["links"][0]["resolution"]["status"],
        "anchor_unknown"
    );
    let policy = value(&run(root.path(), &["policy", "--evaluate"], None));
    assert_eq!(
        policy["files"][0]["suppressions"][0]["states"]["LNK002"]["state"],
        "incomplete"
    );
}

/// Case-insensitive filesystems open the target, but Git and Linux do not, so
/// LNK001 reports the spelling and the anchor stays unchecked on every platform.
#[test]
fn targets_that_differ_in_letter_case_leave_anchors_to_lnk001() {
    let root = workspace(LINKS);
    write(root.path(), "target.md", "# Target\n");
    write(root.path(), "guide.md", "[Missing](TARGET.md#missing)\n");
    let full = run(root.path(), &["check", "--output-format", "json"], None);
    status(&full, 1);
    let report = value(&full);
    assert_eq!(report.as_array().unwrap().len(), 1);
    assert_eq!(report[0]["code"], "LNK001");
    let selected = run(
        root.path(),
        &["check", "target.md", "--output-format", "json"],
        None,
    );
    status(&selected, 0);
    assert_eq!(value(&selected), json!([]));
}

#[cfg(windows)]
#[test]
fn differently_cased_stdin_paths_replace_the_existing_index_entry() {
    let root = workspace(LINKS);
    write(root.path(), "target.md", "# Original\n");
    if !root.path().join("TARGET.md").exists() {
        return;
    }
    write(root.path(), "guide.md", "[Original](target.md#original)\n");
    let expected = run(
        root.path(),
        &[
            "check",
            "--stdin-filename",
            "target.md",
            "--output-format",
            "json",
        ],
        Some("# Replacement\n"),
    );
    status(&expected, 1);
    let alias = run(
        root.path(),
        &[
            "check",
            "--stdin-filename",
            "TARGET.md",
            "--output-format",
            "json",
        ],
        Some("# Replacement\n"),
    );
    status(&alias, 1);
    assert_eq!(alias.stdout, expected.stdout);
    assert_eq!(
        std::fs::read_to_string(root.path().join("target.md")).unwrap(),
        "# Original\n"
    );
}

#[test]
fn moving_identical_bytes_recomputes_kind_and_relative_links() {
    let root = workspace(
        "[[kinds]]\npath='old/**'\nkind='reference'\n[lint]\nselect=['KND001','LNK001']\n",
    );
    let source = "# Source\n\n[Local](target.md)\n";
    write(root.path(), "old/source.md", source);
    write(root.path(), "old/target.md", "# Target\n");
    status(
        &run(
            root.path(),
            &["check", "old/source.md", "--output-format", "json"],
            None,
        ),
        0,
    );
    std::fs::create_dir(root.path().join("new")).unwrap();
    std::fs::rename(
        root.path().join("old/source.md"),
        root.path().join("new/source.md"),
    )
    .unwrap();
    let output = run(
        root.path(),
        &["check", "new/source.md", "--output-format", "json"],
        None,
    );
    status(&output, 1);
    let diagnostics = value(&output);
    let codes: Vec<_> = diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["KND001", "LNK001"]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("new/source.md")).unwrap(),
        source
    );
}

#[test]
fn stdin_target_overlay_affects_upstream_links_without_changing_disk() {
    let root = workspace(LINKS);
    write(root.path(), "target.md", "# Original\n");
    write(root.path(), "guide.md", "[Original](target.md#original)\n");
    let output = run(
        root.path(),
        &[
            "check",
            "--stdin-filename",
            "target.md",
            "--output-format",
            "json",
        ],
        Some("# Replacement\n"),
    );
    status(&output, 1);
    assert_eq!(value(&output)[0]["filename"], "guide.md");
    assert_eq!(value(&output)[0]["code"], "LNK002");
    assert_eq!(
        std::fs::read_to_string(root.path().join("target.md")).unwrap(),
        "# Original\n"
    );
    status(
        &run(root.path(), &["check", "--output-format", "json"], None),
        0,
    );
    status(
        &run(
            root.path(),
            &["check", "--fix", "--stdin-filename", "target.md"],
            Some("# Replacement\n"),
        ),
        2,
    );
}

#[test]
fn stdin_overlay_cannot_enter_an_external_symlinked_directory() {
    let root = workspace(LINKS);
    let external = TempDir::new().unwrap();
    let alias = root.path().join("external");
    #[cfg(unix)]
    std::os::unix::fs::symlink(external.path(), &alias).unwrap();
    #[cfg(windows)]
    match std::os::windows::fs::symlink_dir(external.path(), &alias) {
        Ok(()) => {}
        Err(error)
            if error.raw_os_error() == Some(1314)
                || error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            return;
        }
        Err(error) => panic!("Cannot create symlink: {error}"),
    }
    let output = run(
        root.path(),
        &[
            "check",
            "--stdin-filename",
            "external/new.md",
            "--output-format",
            "json",
        ],
        Some("# Outside overlay\n"),
    );
    status(&output, 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside"));
    assert!(!external.path().join("new.md").exists());
}

#[test]
fn incomplete_full_index_errors_override_selected_exit_zero_and_prohibit_fixes() {
    let root = workspace(LINKS);
    let source =
        "<!-- seiso: allow LNK002 -- Incomplete workspace. -->\n[Self](#guide)\n\n# Guide\n";
    write(root.path(), "guide.md", source);
    std::fs::write(root.path().join("unreadable.md"), [0xff]).unwrap();
    let output = run(
        root.path(),
        &[
            "check",
            "guide.md",
            "--exit-zero",
            "--fix",
            "--statistics",
            "--output-format",
            "json",
        ],
        None,
    );
    status(&output, 2);
    assert_eq!(value(&output)["diagnostics"], json!([]));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unreadable.md"));
    assert_eq!(
        value(&output)["statistics"]["suppressions"][0]["declaration"]["states"]["LNK002"]["state"],
        "incomplete"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("guide.md")).unwrap(),
        source
    );
}

#[test]
fn safe_fix_respects_selection_preserves_disabled_codes_and_is_idempotent_with_crlf() {
    let root = workspace("[lint]\nselect=['LNK001','STL001','SUP002']\n");
    let source = "<!-- seiso: allow LNK001, STL001 -- Historical. -->\r\n\r\nText.\r\n";
    write(root.path(), "a.md", source);
    write(root.path(), "b.md", source);
    let output = run(
        root.path(),
        &["check", "a.md", "--fix", "--output-format", "json"],
        None,
    );
    status(&output, 0);
    let expected = source.replacen("LNK001, ", "", 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.md")).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("b.md")).unwrap(),
        source
    );
    let fixed_path = root.path().join("a.md");
    let original_permissions = std::fs::metadata(&fixed_path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&fixed_path, readonly).unwrap();
    let modified = std::fs::metadata(&fixed_path).unwrap().modified().unwrap();
    let repeated = run(
        root.path(),
        &["check", "a.md", "--fix", "--output-format", "json"],
        None,
    );
    let unchanged = std::fs::metadata(&fixed_path).unwrap().modified().unwrap() == modified;
    std::fs::set_permissions(&fixed_path, original_permissions).unwrap();
    status(&repeated, 0);
    assert!(unchanged);
    assert_eq!(output.stdout, repeated.stdout);
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.md")).unwrap(),
        expected
    );
}

#[test]
fn read_only_fix_failure_preserves_source_and_reports_diagnostics_with_exit_two() {
    let root = workspace("[lint]\nselect=['LNK001','SUP002']\n");
    let source = "<!-- seiso: allow LNK001 -- Historical. -->\n\nText.\n";
    write(root.path(), "guide.md", source);
    let path = root.path().join("guide.md");
    let original = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    let output = run(
        root.path(),
        &["check", "--fix", "--exit-zero", "--output-format", "json"],
        None,
    );
    std::fs::set_permissions(&path, original).unwrap();
    status(&output, 2);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    assert_eq!(value(&output)[0]["code"], "SUP002");
    assert!(String::from_utf8_lossy(&output.stderr).contains("read-only"));
}

#[test]
fn github_statistics_and_tool_errors_cannot_inject_workflow_commands_on_stderr() {
    let root = workspace("[lint]\nselect=['LNK001','SUP002']\n");
    write(
        root.path(),
        "guide.md",
        "<!-- seiso: allow LNK001 -- ##[error]Injected message. -->\n\nText.\n",
    );
    std::fs::write(root.path().join("##[error]unexpected.md"), [0xff]).unwrap();
    let output = run(
        root.path(),
        &["check", "--statistics", "--output-format", "github"],
        None,
    );
    status(&output, 2);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("##["), "{stderr}");
    assert!(
        stderr
            .lines()
            .all(|line| !line.trim_start().starts_with("::")),
        "{stderr}"
    );
    assert!(stderr.contains("Injected message."));
    assert!(stderr.contains("Cannot read UTF-8"));
    let config_error = run(
        root.path(),
        &[
            "check",
            "--config",
            "##[error]missing.toml",
            "--output-format",
            "github",
        ],
        None,
    );
    status(&config_error, 2);
    assert!(!String::from_utf8_lossy(&config_error.stderr).contains("##["));
}

#[test]
fn statistics_include_reasons_states_and_machine_outputs_remain_parseable() {
    let root = workspace("[lint]\nselect=['LNK001','STL001','SUP002']\n");
    write(
        root.path(),
        "guide.md",
        "<!-- seiso: allow LNK001, STL001 -- Historical reason. -->\n\nText.\n",
    );
    let output = run(
        root.path(),
        &["check", "--statistics", "--output-format", "json"],
        None,
    );
    status(&output, 1);
    assert!(output.stderr.is_empty());
    let report = value(&output);
    assert_eq!(report["statistics"]["rules"]["SUP002"], 1);
    let declaration = &report["statistics"]["suppressions"][0]["declaration"];
    assert_eq!(declaration["reason"], "Historical reason.");
    assert_eq!(declaration["states"]["LNK001"]["state"], "stale");
    assert_eq!(declaration["states"]["STL001"]["state"], "rule_disabled");
    let sarif = run(
        root.path(),
        &["check", "--statistics", "--output-format", "sarif"],
        None,
    );
    status(&sarif, 1);
    assert!(sarif.stderr.is_empty());
    let sarif = value(&sarif);
    assert_eq!(sarif["version"], "2.1.0");
    assert_eq!(
        sarif["runs"][0]["properties"]["statistics"],
        report["statistics"]
    );
    assert!(sarif["runs"][0]["results"][0]["fixes"].is_array());
    let github = run(
        root.path(),
        &["check", "--statistics", "--output-format", "github"],
        None,
    );
    status(&github, 1);
    let annotations = String::from_utf8_lossy(&github.stdout);
    assert_eq!(annotations.lines().count(), 1);
    let filename = root.path().join("guide.md");
    // Unix current_dir resolves directory symlinks, including macOS /var.
    #[cfg(unix)]
    let filename = filename.canonicalize().unwrap();
    let filename = filename.to_string_lossy().replace('\\', "/");
    let filename = filename
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C");
    assert!(annotations.starts_with(&format!("::error file={filename},")));
    assert!(!annotations.contains("Rule counts:"));
    assert!(String::from_utf8_lossy(&github.stderr).contains("Historical reason."));
}

#[test]
fn index_dump_is_cwd_independent_and_has_no_absolute_runtime_paths() {
    let root = workspace(LINKS);
    write(
        root.path(),
        "docs/guide.md",
        "# 日本語\n\n[Self](#日本語)\n",
    );
    let top = run(root.path(), &["index", "--dump"], None);
    let nested = run(
        &root.path().join("docs"),
        &["index", "--dump", "--no-cache"],
        None,
    );
    status(&top, 0);
    status(&nested, 0);
    assert_eq!(top.stdout, nested.stdout);
    let index = value(&top);
    assert_eq!(index["index"]["files"][0]["filename"], "docs/guide.md");
    assert_eq!(
        index["index"]["files"][0]["links"][0]["resolution"]["status"],
        "anchor_found"
    );
    assert!(
        !String::from_utf8_lossy(&top.stdout).contains(&root.path().to_string_lossy().to_string())
    );
}

#[test]
fn inspection_commands_do_not_turn_link_resolution_errors_into_lint_failures() {
    let root = workspace(LINKS);
    let target = format!("{}.md", "x".repeat(300));
    write(
        root.path(),
        "guide.md",
        &format!("<!-- seiso: allow-file LNK001 -- External content. -->\n\n[Target]({target})\n"),
    );
    let policy = run(root.path(), &["policy"], None);
    status(&policy, 0);
    assert!(policy.stderr.is_empty());
    assert_eq!(value(&policy)["errors"], json!([]));
    assert_eq!(
        value(&policy)["files"][0]["suppressions"][0]["states"]["LNK001"]["state"],
        "not_evaluated"
    );
    let index = run(root.path(), &["index", "--dump"], None);
    status(&index, 0);
    assert!(index.stderr.is_empty());
    assert_eq!(value(&index)["errors"], json!([]));
    assert_eq!(
        value(&index)["index"]["files"][0]["links"][0]["resolution"]["status"],
        "unreadable"
    );
    let evaluated = run(root.path(), &["policy", "--evaluate"], None);
    status(&evaluated, 2);
    assert!(!value(&evaluated)["errors"].as_array().unwrap().is_empty());
    std::fs::write(root.path().join("unreadable.md"), [0xff]).unwrap();
    for args in [&["policy"][..], &["index", "--dump"][..]] {
        let output = run(root.path(), args, None);
        status(&output, 2);
        assert_eq!(value(&output)["errors"][0]["filename"], "unreadable.md");
    }
}

#[test]
fn per_directory_config_domains_and_languages_limit_duplicate_comparisons() {
    let root = workspace(
        "preview=true\n[[domains]]\npath='other/**'\nname='other'\n[lint]\nselect=['DUP001']\n",
    );
    let definitions = "- `host`: Server address.\n- `port`: Listening port.\n- `user`: Account name.\n- `token`: Access token.\n- `timeout`: Request timeout.\n";
    let document = |kind: &str, language: &str| {
        format!("---\nkind: {kind}\nlang: {language}\n---\n\n{definitions}")
    };
    write(root.path(), "reference.md", &document("reference", "en"));
    write(root.path(), "docs/guide.md", &document("howto", "en"));
    write(root.path(), "docs/.seiso.toml", "extend='../seiso.toml'\n");
    write(root.path(), "translated.md", &document("howto", "zh"));
    write(root.path(), "other/guide.md", &document("howto", "en"));
    let output = run(root.path(), &["check", "--output-format", "json"], None);
    status(&output, 1);
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(diagnostics[0]["filename"], "docs/guide.md");
    assert_eq!(diagnostics[0]["related"][0]["filename"], "reference.md");
    write(
        root.path(),
        "docs/.seiso.toml",
        "extend='../seiso.toml'\n[lint]\nignore=['DUP001']\n",
    );
    let disabled = run(root.path(), &["check", "--output-format", "json"], None);
    status(&disabled, 0);
    assert_eq!(value(&disabled), json!([]));
}

#[test]
fn parallel_reads_keep_output_deterministic_and_continue_after_an_input_error() {
    let root = workspace("[lint]\nselect=['KND001']\n");
    for number in (0..128).rev() {
        write(
            root.path(),
            &format!("pages/{number:03}.md"),
            "# 文档 日本語\n\nA shared paragraph.\n",
        );
    }
    let cold = run(root.path(), &["check", "--output-format", "json"], None);
    let warm = run(root.path(), &["check", "--output-format", "json"], None);
    let disabled = run(
        root.path(),
        &["check", "--no-cache", "--output-format", "json"],
        None,
    );
    status(&cold, 1);
    assert_eq!(cold.stdout, warm.stdout);
    assert_eq!(cold.stdout, disabled.stdout);
    assert_eq!(value(&cold).as_array().unwrap().len(), 128);
    std::fs::write(root.path().join("pages/064.md"), [0xff]).unwrap();
    let partial = run(root.path(), &["check", "--output-format", "json"], None);
    status(&partial, 2);
    assert_eq!(value(&partial).as_array().unwrap().len(), 127);
    assert!(String::from_utf8_lossy(&partial.stderr).contains("pages/064.md"));
}

/// A workspace diagnostic can report a dependency outside the selection, whose
/// single-document rules did not run, so their suppressions stay incomplete.
#[test]
fn unselected_dependencies_report_unchecked_suppressions_as_incomplete() {
    let root = workspace(
        "preview=true\n[[kinds]]\npath='**'\nkind='reference'\n[lint]\nselect=['OWN002','STL001','ORD001']\n",
    );
    let definitions = "- `host`: Server address.\n- `port`: Listening port.\n- `user`: Account name.\n- `token`: Access token.\n- `timeout`: Request timeout.\n";
    write(root.path(), "a.md", &format!("# Settings\n\n{definitions}"));
    write(
        root.path(),
        "b.md",
        &format!(
            "# Settings copy\n\n<!-- seiso: allow STL001 -- Enabled, but b.md is not checked. -->\nCurrently the limit is 50 requests.\n\n<!-- seiso: allow ORD001 -- Reference pages do not enable it. -->\nPlain text.\n\n{definitions}"
        ),
    );
    let output = run(
        root.path(),
        &["check", "a.md", "--statistics", "--output-format", "json"],
        None,
    );
    status(&output, 1);
    let report = value(&output);
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "OWN002" && diagnostic["filename"] == "b.md")
    );
    let states: Vec<_> = report["statistics"]["suppressions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| {
            (
                record["filename"].clone(),
                record["declaration"]["states"].clone(),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            (
                json!("b.md"),
                json!({"STL001": {"state": "incomplete", "count": 0}})
            ),
            (json!("b.md"), json!({"ORD001": {"state": "rule_disabled"}})),
        ]
    );
}
