//! Evaluate the complete rule pipeline against pinned documents and Git inventories.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use seiso::config::{CliOverrides, Config};
use seiso::index::{IndexedFile, InventoryEntryKind, WorkspaceIndex};
use seiso::md::Language;
use seiso::rules::{CheckContext, check};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Batch {
    sources: Vec<Source>,
    #[serde(default)]
    sections: bool,
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
    incomplete_rules: Vec<String>,
    result: seiso::rules::CheckResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    section_annotations: Option<Vec<seiso::sections::SectionAnnotation>>,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: evaluate_m2 INPUT.json OUTPUT.json".into());
    }
    let input: Batch = serde_json::from_slice(&fs::read(&args[1])?)?;
    let workspace = tempfile::tempdir()?;
    let mut output = Vec::new();
    for source in input.sources {
        let root = workspace.path().join(&source.id);
        let config = Config::parse(&source.config, &root)?;
        let inventory = source
            .entries
            .into_iter()
            .map(|entry| {
                let kind = match entry.mode.as_str() {
                    "040000" | "40000" => InventoryEntryKind::Directory,
                    "100644" | "100755" => InventoryEntryKind::File,
                    _ => InventoryEntryKind::Unknown,
                };
                (entry.path, kind)
            })
            .collect();
        let mut files = Vec::new();
        let mut inputs = BTreeMap::new();
        for input in source.documents {
            let raw = fs::read(&input.blob)?;
            if format!("{:x}", Sha256::digest(&raw)) != input.sha256 {
                return Err(format!("input hash mismatch: {}/{}", source.id, input.path).into());
            }
            let document = seiso::md::parse(&String::from_utf8(raw)?)?;
            let path = root.join(&input.path);
            files.push(IndexedFile::new(
                input.path.clone(),
                path,
                document.into(),
                config.clone(),
                &CliOverrides {
                    preview: true,
                    ..CliOverrides::default()
                },
            )?);
            if inputs.insert(input.path, input.sha256).is_some() {
                return Err("duplicate input path".into());
            }
        }
        let index = WorkspaceIndex::new(root.clone(), files, true).with_inventory(inventory);
        let mut cross = seiso::rules::cross_file::check(&index);
        if !cross.errors.is_empty() {
            return Err(format!("cross-file errors: {:?}", cross.errors).into());
        }
        for file in index.files() {
            let context = CheckContext {
                document: &file.document,
                filename: &file.policy.filename,
                path: &file.path,
                workspace_root: &root,
                config: &file.config,
                policy: std::borrow::Cow::Borrowed(&file.policy),
            };
            let mut raw = check(&context, &index);
            raw.diagnostics.extend(
                cross
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.filename == file.policy.filename)
                    .cloned(),
            );
            raw.incomplete_rules.extend(
                cross
                    .incomplete
                    .remove(&file.policy.filename)
                    .unwrap_or_default(),
            );
            let incomplete_rules = raw.incomplete_rules.iter().cloned().collect();
            let result = raw.finish(&file.document, &file.policy.filename);
            if !result.errors.is_empty() {
                return Err(
                    format!("file errors: {}: {:?}", file.policy.filename, result.errors).into(),
                );
            }
            output.push(Output {
                source: source.id.clone(),
                path: file.policy.filename.clone(),
                sha256: inputs[&file.policy.filename].clone(),
                language: file.document.language,
                links: file.document.links.clone(),
                incomplete_rules,
                result,
                section_annotations: input
                    .sections
                    .then(|| seiso::sections::classify(&file.document)),
            });
        }
        eprintln!("evaluated {}: {} documents", source.id, index.files().len());
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
