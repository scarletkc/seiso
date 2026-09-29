use seiso::config::{CliOverrides, Config};
use seiso::index::{IndexedFile, InventoryEntryKind, LinkStatus, WorkspaceIndex};
use seiso::rules::{
    CheckContext, PathStatus, WorkspaceFiles, check, check_raw_with_files, check_with_files,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

struct Inventory {
    paths: BTreeSet<PathBuf>,
}
impl WorkspaceFiles for Inventory {
    fn status(&self, _root: &Path, target: &Path) -> PathStatus {
        if self.paths.contains(target) {
            PathStatus::Exists
        } else {
            PathStatus::Missing
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
        check_with_files(&context, &inventory).unwrap().diagnostics
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
        vec![IndexedFile {
            filename: "docs/new.md".into(),
            path: path.clone(),
            document: document.clone().into(),
            kind: None,
            domain: String::new(),
            enabled_rules: vec!["LNK001".into()],
            config: config.clone(),
        }],
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
    let result = check_raw_with_files(&context, &index).unwrap();
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
        vec![IndexedFile {
            filename: "docs/new.md".into(),
            path: path.clone(),
            document: document.clone().into(),
            kind: None,
            domain: String::new(),
            enabled_rules: vec!["LNK001".into()],
            config: config.clone(),
        }],
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
    let messages: Vec<_> = check_raw_with_files(&context, &index)
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
