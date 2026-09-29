use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use seiso::config::{CliOverrides, Config};
use seiso::md::Language;
use seiso::paths::TargetStatus;
use seiso::rules::{CheckContext, WorkspaceFiles, check};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Batch {
    sources: Vec<Source>,
}
#[derive(Deserialize)]
struct Source {
    id: String,
    config: String,
    entries: Vec<Entry>,
    documents: Vec<Input>,
}
#[derive(Deserialize)]
struct Entry {
    path: String,
    mode: String,
}
#[derive(Deserialize)]
struct Input {
    path: String,
    blob: PathBuf,
    sha256: String,
}
#[derive(Serialize)]
struct Output {
    source: String,
    path: String,
    sha256: String,
    language: Language,
    links: Vec<seiso::md::RawLink>,
    result: seiso::rules::CheckResult,
}
struct Inventory {
    entries: BTreeMap<String, String>,
}
impl WorkspaceFiles for Inventory {
    fn status(&self, root: &Path, target: &Path) -> TargetStatus {
        let Ok(relative) = target.strip_prefix(root) else {
            return TargetStatus::Unknown;
        };
        if relative.as_os_str().is_empty() {
            return TargetStatus::File;
        }
        for ancestor in relative.ancestors() {
            let key = ancestor.to_string_lossy().replace('\\', "/");
            if self
                .entries
                .get(&key)
                .is_some_and(|mode| mode == "120000" || mode == "160000")
            {
                return TargetStatus::Unknown;
            }
        }
        let key = relative.to_string_lossy().replace('\\', "/");
        if self.entries.contains_key(&key) {
            TargetStatus::File
        } else {
            TargetStatus::Missing
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: evaluate INPUT.json OUTPUT.json".into());
    }
    let input: Batch = serde_json::from_slice(&fs::read(&args[1])?)?;
    let workspace = tempfile::tempdir()?;
    let mut output = Vec::new();
    for source in input.sources {
        let root = workspace.path().join(&source.id);
        let config = Config::parse(&source.config, &root)?;
        let inventory = Inventory {
            entries: source
                .entries
                .into_iter()
                .map(|entry| (entry.path, entry.mode))
                .collect(),
        };
        for input in source.documents {
            let raw = fs::read(&input.blob)?;
            if format!("{:x}", Sha256::digest(&raw)) != input.sha256 {
                return Err(format!("input hash mismatch: {}/{}", source.id, input.path).into());
            }
            let text = String::from_utf8(raw)?;
            let document = seiso::md::parse(&text)?;
            let path = root.join(&input.path);
            let context = CheckContext::new(
                &document,
                &input.path,
                &path,
                &root,
                &config,
                &CliOverrides {
                    preview: true,
                    ..CliOverrides::default()
                },
            )?;
            let result = check(&context, &inventory).finish(&document, &input.path);
            if !result.errors.is_empty() {
                return Err(format!("incomplete file: {}: {:?}", input.path, result.errors).into());
            }
            output.push(Output {
                source: source.id.clone(),
                path: input.path,
                sha256: input.sha256,
                language: document.language,
                links: document.links,
                result,
            });
        }
    }
    output.sort_by(|a, b| (&a.source, &a.path).cmp(&(&b.source, &b.path)));
    fs::write(&args[2], serde_json::to_vec(&output)?)?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("evaluation: {error}");
        std::process::exit(2);
    }
}
