//! Opt-in diagnostics for configuration-chain and directory-discovery costs.
//!
//! Run with `cargo test --release --test config_frame_perf -- --ignored --nocapture`.
//! All generated fixtures stay under this worktree's `.temp` directory.

use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use seiso::analysis;
use seiso::config::CliOverrides;
use seiso::config::{Config, Workspace};
use seiso::workspace::{LoadOptions, LoadScope, load};

/// Remove the one fixture tree created by this process, including on panic.
struct Fixture(PathBuf);

impl Fixture {
    /// Allocate a unique fixture inside the repository's `.temp` directory.
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".temp")
            .join(format!("config-frame-perf-{}-{seed}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    /// Write a configuration below the fixture root.
    fn write(&self, path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Verify the final resolved target before recursive deletion, including
        // on Windows where a fixture component could have become a junction.
        let temp = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".temp")
            .canonicalize()
            .unwrap();
        let target = self.0.canonicalize().unwrap();
        assert!(target.starts_with(&temp) && target != temp);
        fs::remove_dir_all(target).unwrap();
    }
}

/// Count content entries without including cache metadata files.
fn cache_entries(directory: &Path) -> usize {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "cache"))
        .count()
}

/// Report all repetitions so the reader can assess scheduler noise.
fn measure(name: &str, operations: usize, mut run: impl FnMut()) {
    let mut samples = Vec::<Duration>::new();
    for _ in 0..3 {
        let start = Instant::now();
        for _ in 0..operations {
            run();
        }
        samples.push(start.elapsed());
    }
    println!(
        "{name},{operations},{}",
        samples
            .iter()
            .map(|d| d.as_micros().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
}

/// Compare inheritance depth, glob compilation, discovery depth, and matching.
#[test]
#[ignore = "performance diagnostic; run deliberately with --ignored --nocapture"]
fn configuration_costs() {
    let fixture = Fixture::new();
    println!("scenario,operations,run1_us,run2_us,run3_us");

    for depth in [1, 8, 64, 128] {
        let mut dir = fixture.0.join(format!("chain-{depth}"));
        for layer in 0..depth {
            let content = if layer == 0 {
                "include = ['**/*.md']\n".to_owned()
            } else {
                "extend = '../seiso.toml'\n".to_owned()
            };
            fixture.write(&dir.join("seiso.toml"), &content);
            if layer + 1 < depth {
                dir = dir.join(format!("d{layer}"));
            }
        }
        let path = dir.join("seiso.toml");
        measure(&format!("load_chain_{depth}"), 30, || {
            black_box(Config::load(&path).unwrap());
        });
    }

    // Same path depth and TOML content as the 128-file chain, but the
    // highest-priority filename needs no candidate-existence metadata probe
    // when an inheritance edge is classified as a governing ancestor.
    let mut dir = fixture.0.join("chain-128-high-priority");
    for layer in 0..128 {
        let content = if layer == 0 {
            "include = ['**/*.md']\n".to_owned()
        } else {
            "extend = '../.seiso.toml'\n".to_owned()
        };
        fixture.write(&dir.join(".seiso.toml"), &content);
        if layer < 127 {
            dir = dir.join(format!("d{layer}"));
        }
    }
    let high_priority = dir.join(".seiso.toml");
    measure("load_chain_128_high_priority", 30, || {
        black_box(Config::load(&high_priority).unwrap());
    });

    for count in [0, 64, 256] {
        let dir = fixture.0.join(format!("patterns-{count}"));
        let content = (0..count)
            .map(|n| format!("[[kinds]]\npath = 'docs/k{n}/**'\nkind = 'reference'\n"))
            .collect::<String>();
        fixture.write(&dir.join("seiso.toml"), &content);
        measure(&format!("load_patterns_{count}"), 30, || {
            black_box(Config::load(&dir.join("seiso.toml")).unwrap());
        });
        let config = Config::load(&dir.join("seiso.toml")).unwrap();
        let miss = dir.join("docs/missing/page.md");
        measure(&format!("match_kind_miss_{count}"), 5000, || {
            black_box(config.kind_for(&miss));
        });
        if count == 256 {
            let files = (0..10_000)
                .map(|n| dir.join(format!("docs/missing/page{n}.md")))
                .collect::<Vec<_>>();
            measure("select_10000_paths_256_patterns", 1, || {
                for file in &files {
                    black_box(config.includes(file));
                    black_box(config.kind_for(file));
                }
            });
        }
    }

    for depth in [1, 8, 32] {
        let dir = fixture.0.join(format!("discovery-{depth}"));
        fixture.write(&dir.join("seiso.toml"), "");
        let workspace = Workspace::discover(&dir, None).unwrap();
        let mut leaf = dir.clone();
        for layer in 0..depth {
            leaf = leaf.join(format!("d{layer}"));
        }
        fs::create_dir_all(&leaf).unwrap();
        let file = leaf.join("page.md");
        measure(&format!("discover_ancestor_{depth}"), 1000, || {
            black_box(workspace.config_for(&file).unwrap());
        });
    }

    // Unlike the repeated-call diagnostic above, these are distinct leaf
    // directories and therefore mimic the cache misses in workspace::load.
    let root = fixture.0.join("distinct-leaves");
    fixture.write(&root.join("seiso.toml"), "");
    let workspace = Workspace::discover(&root, None).unwrap();
    let paths = (0..128)
        .map(|n| {
            let dir = root.join(format!("group{n}")).join("nested");
            fs::create_dir_all(&dir).unwrap();
            dir.join("page.md")
        })
        .collect::<Vec<_>>();
    measure("discover_128_distinct_leaves_depth_2", 1, || {
        for path in &paths {
            black_box(workspace.config_for(path).unwrap());
        }
    });

    // Each selected child uses the same shared source but a different policy
    // base. A source-only cache would be wrong even if it is fast.
    let root = fixture.0.join("shared-template");
    let template = (0..64)
        .map(|n| format!("[[kinds]]\npath = 'docs/k{n}/**'\nkind = 'reference'\n"))
        .collect::<String>();
    fixture.write(&root.join("templates/base.toml"), &template);
    let children = (0..32)
        .map(|n| {
            let path = root.join(format!("child{n}/seiso.toml"));
            fixture.write(&path, "extend = '../templates/base.toml'\n");
            path
        })
        .collect::<Vec<_>>();
    measure("load_32_children_shared_64_pattern_template", 1, || {
        for path in &children {
            black_box(Config::load(path).unwrap());
        }
    });

    // The workspace resolver can avoid discovery I/O, but public `config_for`
    // still returns an owned Config. This isolates clone cost for a large
    // effective policy over many distinct file directories.
    let root = fixture.0.join("large-policy-clones");
    let policy = (0..256)
        .map(|n| format!("[[kinds]]\npath = 'docs/k{n}/**'\nkind = 'reference'\n"))
        .collect::<String>();
    fixture.write(&root.join("seiso.toml"), &policy);
    let workspace = Workspace::discover(&root, None).unwrap();
    let paths = (0..128)
        .map(|n| {
            let dir = root.join(format!("docs/group{n}"));
            fs::create_dir_all(&dir).unwrap();
            dir.join("page.md")
        })
        .collect::<Vec<_>>();
    measure("resolve_128_distinct_dirs_256_patterns", 1, || {
        for path in &paths {
            black_box(workspace.config_for(path).unwrap());
        }
    });
}

/// Exercise the child-invocation project pass without CLI rendering noise.
///
/// The two rule selections differ only in whether they require cross-file
/// indexing. Fixtures are created outside the timed calls; cold means the
/// parse cache is empty, not that the operating-system page cache is cold.
#[test]
#[ignore = "performance diagnostic; run deliberately with --ignored --nocapture"]
fn child_project_dependency_costs() {
    for nested_configs in [0, 8] {
        let fixture = Fixture::new();
        let root = fixture.0.join(format!("child-project-{nested_configs}"));
        fixture.write(&root.join("seiso.toml"), "include = ['**/*.md']\n");
        fixture.write(&root.join("docs/seiso.toml"), "extend = '../seiso.toml'\n");
        fixture.write(
            &root.join("docs/topic/seiso.toml"),
            "extend = '../seiso.toml'\n",
        );
        fixture.write(
            &root.join("docs/topic/guide.md"),
            "# Guide\n\nSee [page](../../reference/group0/page0.md#page-0-0).\n",
        );
        for group in 0..32 {
            if group < nested_configs {
                fixture.write(
                    &root.join(format!("reference/group{group}/seiso.toml")),
                    "extend = '../../seiso.toml'\n",
                );
            }
            for page in 0..32 {
                fixture.write(
                    &root.join(format!("reference/group{group}/page{page}.md")),
                    &format!("# Page {group}-{page}\n\nA small project dependency.\n"),
                );
            }
        }
        let child = root.join("docs/topic");
        println!(
            "scenario,nested_configs,run,load_us,analysis_us,index_docs,selected_docs,diagnostics,child_cache_entries,project_cache_entries"
        );
        let cases = [
            (&child, "guide.md", "KND001", false, "child_single_file"),
            (
                &root,
                "docs/topic/guide.md",
                "KND001",
                false,
                "root_single_file",
            ),
            (&child, "guide.md", "LNK002", false, "child_cross_file"),
            (
                &root,
                "docs/topic/guide.md",
                "LNK002",
                false,
                "root_cross_file",
            ),
            (
                &child,
                "guide.md",
                "LNK002",
                true,
                "child_cross_file_no_cache",
            ),
        ];
        for (cwd, requested, selector, no_cache, label) in cases {
            for run in 0..2 {
                let options = LoadOptions {
                    paths: vec![PathBuf::from(requested)],
                    overrides: CliOverrides {
                        select: Some(vec![selector.to_owned()]),
                        preview: true,
                        ..CliOverrides::default()
                    },
                    no_cache,
                    ..LoadOptions::default()
                };
                let start = Instant::now();
                let snapshot = load(cwd, &options, LoadScope::Check).unwrap();
                let elapsed = start.elapsed();
                assert!(snapshot.errors.is_empty(), "{:?}", snapshot.errors);
                let index_docs = snapshot.index.files().len();
                let selected_docs = snapshot.selected.len();
                let expected = if selector == "KND001" { 1 } else { 1025 };
                assert_eq!(index_docs, expected);
                assert_eq!(selected_docs, 1);
                let analysis_start = Instant::now();
                let analysis = analysis::check(snapshot, &options.overrides).unwrap();
                let analysis_us = analysis_start.elapsed().as_micros();
                println!(
                    "{label},{nested_configs},{run},{},{analysis_us},{index_docs},{selected_docs},{},{},{}",
                    elapsed.as_micros(),
                    analysis.diagnostics.len(),
                    cache_entries(&child.join(".seiso_cache")),
                    cache_entries(&root.join(".seiso_cache"))
                );
            }
        }
    }
}
