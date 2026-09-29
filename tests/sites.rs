use common::{value, write};
mod common;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

fn run(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(root)
        .args(arguments)
        .output()
        .unwrap()
}

const LINKS: &str = "preview = true\n\n[lint]\nselect = ['LNK001', 'LNK002']\n";

const PAGE: &str = "---\nkind: howto\n---\n# Start\n\n\
[md](/guide/setup) [mdx](/guide/tools) [index](/guide/) [readme](/reference)\n\n\
[html](setup.html) [mdbook index](../reference/index.html) [public](/logo.png)\n\n\
[mdx index](/components/)\n\n\
[anchor](/guide/#install) [page anchor](/guide/setup#steps)\n\n\
[missing anchor](/guide/setup#removed)\n\n\
[missing route](/missing)\n\n\
[outside the site](../../docs/outside)\n";

/// A site in `site/` whose pages link by route, beside repository documents.
fn site_workspace(sites: &str) -> TempDir {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    write(root, "seiso.toml", &format!("{LINKS}{sites}"));
    write(root, "site/guide/start.md", PAGE);
    write(root, "site/guide/index.md", "# Guide\n\n## Install\n");
    write(root, "site/guide/setup.md", "# Setup\n\n## Steps\n");
    write(root, "site/guide/tools.mdx", "# Tools\n");
    write(root, "site/reference/README.md", "# Reference\n");
    write(root, "site/components/index.mdx", "# Components\n");
    write(root, "site/public/logo.png", "image");
    write(root, "docs/outside.md", "# Outside\n");
    workspace
}

const SITE: &str = "\n[[sites]]\npath = 'site/**'\nroot = 'site'\npublic = 'site/public'\n";

fn diagnostics(root: &Path) -> Vec<(String, String, String)> {
    let output = run(root, &["check", "--output-format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    value(&output)
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap().to_owned(),
                diagnostic["message"].as_str().unwrap().to_owned(),
                diagnostic["suggestion"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn site_documents_resolve_routes_to_page_sources_and_public_files() {
    let workspace = site_workspace(SITE);
    let found = diagnostics(workspace.path());
    let summary: Vec<_> = found
        .iter()
        .map(|(code, message, _)| (code.as_str(), message.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (
                "LNK002",
                "Anchor \"removed\" does not exist in site/guide/setup.md."
            ),
            (
                "LNK001",
                "Local link target \"/missing\" does not exist in the workspace."
            ),
            (
                "LNK001",
                "Local link target \"../../docs/outside\" does not exist in the workspace."
            ),
        ]
    );
    assert_eq!(
        found[1].2,
        "Update the path or restore the target; the site rooted at \"site\" has no page or file for this route either."
    );
}

#[test]
fn without_a_site_entry_route_links_stay_missing_repository_paths() {
    let workspace = site_workspace("");
    let found = diagnostics(workspace.path());
    assert_eq!(
        found.iter().filter(|(code, ..)| code == "LNK001").count(),
        13
    );
    assert!(found.iter().all(|(code, ..)| code == "LNK001"));
    assert!(found.iter().all(|(.., suggestion)| suggestion == "Update the path or restore the target; paths resolve from this document's directory, or from the workspace root when they start with /, and seiso does not add .md or index.md."));
}

#[test]
fn documents_outside_the_site_path_keep_repository_resolution() {
    let workspace = site_workspace(SITE);
    let root = workspace.path();
    write(
        root,
        "README.md",
        "---\nkind: readme\n---\n# Project\n\n[route](/guide/setup) [path](site/guide/setup.md)\n",
    );
    let output = run(root, &["check", "README.md", "--output-format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1);
    assert_eq!(
        diagnostics[0]["message"],
        "Local link target \"/guide/setup\" does not exist in the workspace."
    );
}

#[test]
fn base_is_stripped_from_root_relative_links_only_when_present() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    write(
        root,
        "seiso.toml",
        &format!("{LINKS}\n[[sites]]\npath = 'site/**'\nroot = 'site'\nbase = '/docs/'\n"),
    );
    write(root, "site/index.md", "# Home\n");
    write(root, "site/guide/setup.md", "# Setup\n");
    write(
        root,
        "site/guide/start.md",
        "---\nkind: howto\n---\n# Start\n\n[with base](/docs/guide/setup) [base root](/docs) [without base](/guide/setup)\n",
    );
    let output = run(root, &["check", "--output-format", "json"]);
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1, "{diagnostics}");
    assert_eq!(
        diagnostics[0]["message"],
        "Local link target \"/guide/setup\" does not exist in the workspace."
    );
}

#[test]
fn a_trailing_slash_prefers_the_directory_index_over_a_same_named_page() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    write(root, "seiso.toml", &format!("{LINKS}{SITE}"));
    write(root, "site/guide.md", "# Guide page\n");
    write(root, "site/guide/index.md", "# Guide\n\n## Install\n");
    write(root, "site/other.md", "# Other\n");
    write(
        root,
        "site/start.md",
        "---\nkind: howto\n---\n# Start\n\n[index](/guide/#install) [page](/guide#install) [same-named page](/other/)\n",
    );
    let output = run(root, &["check", "--output-format", "json"]);
    let diagnostics = value(&output);
    assert_eq!(diagnostics.as_array().unwrap().len(), 1, "{diagnostics}");
    assert_eq!(
        diagnostics[0]["message"],
        "Anchor \"install\" does not exist in site/guide.md."
    );
    assert_eq!(diagnostics[0]["location"]["column"], 26);
}

#[test]
fn the_written_path_wins_over_a_route_and_the_dump_shows_resolved_targets() {
    let workspace = site_workspace(SITE);
    let root = workspace.path();
    write(root, "site/api", "plain file");
    write(root, "site/api.md", "# API\n");
    write(
        root,
        "site/guide/start.md",
        "---\nkind: howto\n---\n# Start\n\n[physical](../api#absent) [route](/api.html#absent) [index](/guide/#install)\n",
    );
    write(
        root,
        "docs/outside.md",
        "# Outside\n\n[route](/guide/setup)\n",
    );
    let output = run(root, &["index", "--dump"]);
    assert_eq!(output.status.code(), Some(0));
    let index = value(&output)["index"].clone();
    let file = |name: &str| {
        index["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["filename"] == name)
            .unwrap()
            .clone()
    };
    let start = file("site/guide/start.md");
    assert_eq!(
        start["site"],
        json!({"path": "site/**", "root": "site", "public": "site/public", "base": "/"})
    );
    let resolutions: Vec<_> = start["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| {
            (
                link["resolution"]["target"].as_str().unwrap().to_owned(),
                link["resolution"]["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        resolutions,
        [
            ("site/api".into(), "anchor_unknown".into()),
            ("site/api.md".into(), "anchor_missing".into()),
            ("site/guide/index.md".into(), "anchor_found".into()),
        ]
    );
    let outside = file("docs/outside.md");
    assert!(outside.get("site").is_none());
    assert_eq!(outside["links"][0]["resolution"]["status"], "missing");
}

/// Some generators lowercase page routes, so `/guide/contributing` can serve
/// `CONTRIBUTING.md`; no generator serves `/guide/Setup` for `setup.md`, and
/// files served under their own names keep their case.
#[test]
fn only_a_lowercase_route_reaches_a_page_whose_name_differs_in_letter_case() {
    let workspace = site_workspace(SITE);
    let root = workspace.path();
    write(
        root,
        "site/guide/CONTRIBUTING.md",
        "# Contributing\n\n## Setup\n",
    );
    write(root, "site/public/Banner.png", "image");
    write(root, "site/assets/Diagram.svg", "image");
    write(
        root,
        "site/guide/start.md",
        "---\nkind: howto\n---\n# Start\n\n[route](/guide/contributing#setup) [anchor](/guide/contributing#removed)\n\n[route case](/guide/Setup)\n\n[path case](Setup.md)\n\n![public](/banner.png) ![root](/assets/diagram.svg)\n",
    );
    let found = diagnostics(root);
    let summary: Vec<_> = found
        .iter()
        .map(|(code, message, _)| (code.as_str(), message.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (
                "LNK002",
                "Anchor \"removed\" does not exist in site/guide/CONTRIBUTING.md."
            ),
            (
                "LNK001",
                "Local link target \"/guide/Setup\" differs in letter case from \"site/guide/setup.md\"."
            ),
            (
                "LNK001",
                "Local link target \"Setup.md\" differs in letter case from \"site/guide/setup.md\"."
            ),
            (
                "LNK001",
                "Local link target \"/banner.png\" differs in letter case from \"site/public/Banner.png\"."
            ),
            (
                "LNK001",
                "Local link target \"/assets/diagram.svg\" differs in letter case from \"site/assets/Diagram.svg\"."
            ),
        ]
    );
}

#[test]
fn policy_names_each_file_site() {
    let workspace = site_workspace(SITE);
    let output = run(workspace.path(), &["policy"]);
    assert_eq!(output.status.code(), Some(0));
    let report = value(&output);
    let sites: Vec<_> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["filename"].as_str().unwrap(),
                file["site"]["root"].clone(),
            )
        })
        .collect();
    assert!(sites.contains(&("site/guide/start.md", json!("site"))));
    assert!(sites.contains(&("docs/outside.md", Value::Null)));
    assert_eq!(
        report["configurations"]["seiso.toml"]["sites"][0]["public"],
        "site/public"
    );
}

#[test]
fn inspection_output_has_no_site_fields_without_site_entries() {
    let workspace = site_workspace("");
    for arguments in [&["policy"][..], &["index", "--dump"]] {
        let output = run(workspace.path(), arguments);
        assert_eq!(output.status.code(), Some(0));
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("\"site\""), "{text}");
        assert!(!text.contains("\"sites\""), "{text}");
    }
}

#[test]
fn a_nested_configuration_can_name_a_site_root_above_it() {
    let workspace = TempDir::new().unwrap();
    let root = workspace.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    write(root, "seiso.toml", LINKS);
    write(
        root,
        "docs/seiso.toml",
        &format!("{LINKS}\n[[sites]]\npath = '**'\nroot = '../site'\n"),
    );
    write(root, "site/guide.md", "# Guide\n");
    write(
        root,
        "docs/start.md",
        "---\nkind: howto\n---\n# Start\n\n[Guide](/guide)\n",
    );
    let output = run(root, &["check", "docs"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn invalid_site_entries_are_configuration_errors() {
    for (entry, expected) in [
        (
            "path = 'site/**'\nroot = '/site'",
            "sites.root \"/site\" must be a directory relative to this configuration",
        ),
        (
            "path = 'site/**'\nroot = 'site'\npublic = ''",
            "sites.public \"\" must be a directory relative to this configuration",
        ),
        (
            "path = 'site/**'\nroot = 'site'\nbase = 'guide/'",
            "sites.base \"guide/\" must be a URL path that starts with /",
        ),
        (
            "path = 'site/**'\nroot = '../outside'",
            "sites.root \"../outside\" is outside workspace",
        ),
        (
            "path = 'site/**'\nroot = 'site'\npublic = 'site/../../public'",
            "sites.public \"site/../../public\" is outside workspace",
        ),
        ("path = 'site/**'", "missing field `root`"),
        (
            "path = 'site/**'\nroot = 'site'\nsrc = 'x'",
            "unknown field `src`",
        ),
    ] {
        let workspace = TempDir::new().unwrap();
        write(
            workspace.path(),
            "seiso.toml",
            &format!("[[sites]]\n{entry}\n"),
        );
        write(workspace.path(), "a.md", "# A\n");
        let output = run(workspace.path(), &["check"]);
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
    }
}
