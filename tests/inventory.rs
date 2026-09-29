mod common;
use common::{CheckContext, check};

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use seiso::config::{CliOverrides, Config};
use seiso::index::{IndexedFile, InventoryEntryKind, LinkStatus, WorkspaceIndex};
use seiso::paths::TargetStatus;
use seiso::rules::WorkspaceFiles;

struct Inventory {
    paths: BTreeSet<PathBuf>,
}
impl WorkspaceFiles for Inventory {
    fn status(&self, _root: &Path, target: &Path) -> TargetStatus {
        if self.paths.contains(target) {
            TargetStatus::File
        } else {
            TargetStatus::Missing
        }
    }
}

#[test]
fn frozen_inventory_uses_the_same_path_resolution_as_the_filesystem() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("配置.md"), "# Configuration").unwrap();
    let document = seiso::md::parse("[existing](/%E9%85%8D%E7%BD%AE.md#anchor)\n\n[missing](missing.md)\n\n[outside](../../outside.md)\n").unwrap();
    let config = Config::parse("preview=true\n[lint]\nselect=['LNK001']", root.path()).unwrap();
    let context = CheckContext {
        document: &document,
        filename: "docs/test.md",
        path: &root.path().join("docs/test.md"),
        workspace_root: root.path(),
        config: &config,
        overrides: &CliOverrides::default(),
    };
    let inventory = Inventory {
        paths: [root.path().join("配置.md")].into(),
    };
    assert_eq!(
        check(&context).unwrap().diagnostics,
        context
            .check(&inventory)
            .unwrap()
            .finish(context.document, context.filename)
            .diagnostics
    );
    assert_eq!(check(&context).unwrap().diagnostics.len(), 1);
}

#[test]
fn index_inventory_supplies_file_checks_without_a_materialized_workspace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("snapshot");
    let document = seiso::md::parse("# New\n\n[self](new.md#new)\n\n[file](/%E9%85%8D%E7%BD%AE.md#%ZZ)\n\n[directory](/docs)\n\n[missing](missing.md#%ZZ)\n\n[unknown](/submodule/missing.md)\n\n[outside](../../outside.md)\n").unwrap();
    let config = Config::parse("preview=true\n[lint]\nselect=['LNK001']", &root).unwrap();
    let path = root.join("docs/new.md");
    let index = WorkspaceIndex::new(
        root.clone(),
        vec![
            IndexedFile::new(
                "docs/new.md".into(),
                path.clone(),
                document.clone().into(),
                config.clone(),
                &CliOverrides::default(),
            )
            .unwrap(),
        ],
        true,
    )
    .with_inventory(BTreeMap::from([
        ("配置.md".into(), InventoryEntryKind::File),
        ("docs".into(), InventoryEntryKind::Directory),
        ("submodule".into(), InventoryEntryKind::Unknown),
    ]));
    let context = CheckContext {
        document: &document,
        filename: "docs/new.md",
        path: &path,
        workspace_root: &root,
        config: &config,
        overrides: &CliOverrides::default(),
    };
    let result = context.check(&index).unwrap();
    assert!(result.errors.is_empty());
    assert_eq!(result.diagnostics.len(), 1);
    assert!(result.diagnostics[0].message.contains("missing.md#%ZZ"));
    assert!(result.incomplete_rules.contains("LNK001"));
    assert_eq!(
        index.resolve_link("docs/new.md", "new.md#new").status,
        LinkStatus::AnchorFound
    );
    assert_eq!(
        index.resolve_link("docs/new.md", "/配置.md#%ZZ").status,
        LinkStatus::AnchorUnknown
    );
    assert!(!root.exists());
}

#[test]
fn frozen_inventories_compare_letter_case_like_the_filesystem() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("snapshot");
    let document = seiso::md::parse(
        "[path](Setup.md#install)\n\n[route](/docs/contributing#setup)\n\n[route case](/docs/Setup)\n",
    )
    .unwrap();
    let config = Config::parse(
        "[lint]\nselect=['LNK001']\n[[sites]]\npath='docs/**'\nroot='docs'\nbase='/docs/'\n",
        &root,
    )
    .unwrap();
    let path = root.join("docs/new.md");
    let index = WorkspaceIndex::new(
        root.clone(),
        vec![
            IndexedFile::new(
                "docs/new.md".into(),
                path.clone(),
                document.clone().into(),
                config.clone(),
                &CliOverrides::default(),
            )
            .unwrap(),
        ],
        true,
    )
    .with_inventory(BTreeMap::from([
        ("docs".into(), InventoryEntryKind::Directory),
        ("docs/CONTRIBUTING.md".into(), InventoryEntryKind::File),
        ("docs/setup.md".into(), InventoryEntryKind::File),
    ]));
    let context = CheckContext {
        document: &document,
        filename: "docs/new.md",
        path: &path,
        workspace_root: &root,
        config: &config,
        overrides: &CliOverrides::default(),
    };
    let messages: Vec<_> = context
        .check(&index)
        .unwrap()
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect();
    assert_eq!(
        messages,
        [
            "Local link target \"Setup.md#install\" differs in letter case from \"docs/setup.md\".",
            "Local link target \"/docs/Setup\" differs in letter case from \"docs/setup.md\".",
        ]
    );
    let route = index.resolve_link("docs/new.md", "/docs/contributing#setup");
    assert_eq!(route.target.as_deref(), Some("docs/CONTRIBUTING.md"));
    assert_eq!(route.status, LinkStatus::AnchorUnknown);
    assert_eq!(
        index.resolve_link("docs/new.md", "Setup.md#install").status,
        LinkStatus::Missing
    );
}

#[test]
fn file_existence_ignores_unresolved_fragments_and_queries() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("target.md"), "# Target\n").unwrap();
    let config = Config::parse("[lint]\nselect=['LNK001','SUP002']", root.path()).unwrap();
    let path = root.path().join("source.md");
    for destination in [
        "target.md#${anchor}",
        "target.md#%ZZ",
        "target.md?q=${query}",
        "target.md?q=%ZZ",
    ] {
        let document = seiso::md::parse(&format!(
            "<!-- seiso: allow LNK001 -- The target was previously absent. -->\n[Target]({destination})\n"
        )).unwrap();
        let context = CheckContext {
            document: &document,
            filename: "source.md",
            path: &path,
            workspace_root: root.path(),
            config: &config,
            overrides: &CliOverrides::default(),
        };
        let result = check(&context).unwrap();
        assert!(result.errors.is_empty(), "{destination}");
        assert_eq!(result.diagnostics.len(), 1, "{destination}");
        assert_eq!(result.diagnostics[0].code, "SUP002", "{destination}");
        assert!(
            matches!(
                result.suppressions[0].states["LNK001"],
                seiso::rules::suppression::SuppressionState::Stale
            ),
            "{destination}"
        );
    }
}

#[test]
fn workspace_file_status_preserves_directories_and_outside_paths() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    let index = WorkspaceIndex::new(root.path().to_path_buf(), Vec::new(), true);
    assert_eq!(
        index.status(root.path(), &root.path().join("docs")),
        TargetStatus::Directory
    );
    assert_eq!(
        index.status(root.path(), root.path().parent().unwrap()),
        TargetStatus::OutsideWorkspace
    );
    let local = seiso::rules::LocalWorkspaceFiles::default();
    assert_eq!(
        local.status(root.path(), &root.path().join("docs")),
        TargetStatus::Directory
    );
    assert_eq!(
        local.status(root.path(), root.path().parent().unwrap()),
        TargetStatus::OutsideWorkspace
    );
}

#[test]
fn directory_routes_complete_existence_checks_before_page_candidates() {
    struct RouteInventory;
    impl WorkspaceFiles for RouteInventory {
        fn status(&self, root: &Path, target: &Path) -> TargetStatus {
            if target == root.join("site/guide") {
                TargetStatus::Directory
            } else if target == root.join("site/guide/index.md") {
                TargetStatus::Unreadable("page source is unreadable".into())
            } else {
                TargetStatus::Missing
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let document = seiso::md::parse("[Guide](/guide/)\n").unwrap();
    let config = Config::parse(
        "[lint]\nselect=['LNK001']\n[[sites]]\npath='site/**'\nroot='site'\n",
        root.path(),
    )
    .unwrap();
    let context = CheckContext {
        document: &document,
        filename: "site/from.md",
        path: &root.path().join("site/from.md"),
        workspace_root: root.path(),
        config: &config,
        overrides: &CliOverrides::default(),
    };
    let result = context.check(&RouteInventory).unwrap();
    assert!(result.diagnostics.is_empty());
    assert!(result.errors.is_empty());
    assert!(result.incomplete_rules.is_empty());
}

#[test]
fn index_rules_stay_disabled_until_workspace_results_are_merged() {
    let root = tempfile::tempdir().unwrap();
    let source = "---\nkind: reference\n---\n<!-- seiso: allow LNK002 -- The generated heading is added later. -->\n[Heading](#missing)\n";
    let config = Config::parse(
        "preview = true\n[lint]\nselect = ['LNK002', 'SUP002']\n",
        root.path(),
    )
    .unwrap();
    let path = root.path().join("page.md");
    let document = seiso::md::parse(source).unwrap();
    let index = WorkspaceIndex::new(
        root.path().to_path_buf(),
        vec![
            IndexedFile::new(
                "page.md".into(),
                path.clone(),
                document.clone().into(),
                config.clone(),
                &CliOverrides::default(),
            )
            .unwrap(),
        ],
        true,
    );
    let file = &index.files()[0];
    let indexed = seiso::rules::check(
        &seiso::rules::CheckContext::indexed(file, root.path()),
        &index,
    )
    .finish(file.document(), file.filename());
    let standalone = check(&CheckContext {
        document: &document,
        filename: "page.md",
        path: &path,
        workspace_root: root.path(),
        config: &config,
        overrides: &CliOverrides::default(),
    })
    .unwrap();
    // An unmerged LNK002 would leave the suppression unused and report SUP002.
    assert_eq!(indexed.enabled_rules, ["SUP002"]);
    assert_eq!(standalone.enabled_rules, indexed.enabled_rules);
    assert!(indexed.diagnostics.is_empty(), "{:?}", indexed.diagnostics);
}
