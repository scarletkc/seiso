//! Check frozen document revisions without executing their repositories.

use std::fs;

use seiso::config::{CliOverrides, Config};
use seiso::rules::{CheckContext, LocalWorkspaceFiles, check};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Batch {
    config: String,
    documents: Vec<Input>,
}
#[derive(Deserialize)]
struct Input {
    id: String,
    path: String,
    source: String,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: replay_m3 INPUT.json OUTPUT.json".into());
    }
    let batch: Batch = serde_json::from_slice(&fs::read(&args[1])?)?;
    let workspace = tempfile::tempdir()?;
    let config = Config::parse(&batch.config, workspace.path())?;
    let mut rows = Vec::new();
    for input in batch.documents {
        let document = seiso::md::parse(&input.source)?;
        let path = workspace.path().join(&input.path);
        let context = CheckContext::new(
            &document,
            &input.path,
            &path,
            workspace.path(),
            &config,
            &CliOverrides {
                preview: true,
                ..CliOverrides::default()
            },
        )?;
        let raw = check(&context, &LocalWorkspaceFiles::default());
        let incomplete_rules: Vec<_> = raw.incomplete_rules.iter().cloned().collect();
        let result = raw.finish(&document, &input.path);
        if !result.errors.is_empty() {
            return Err(format!("input {}: {:?}", input.id, result.errors).into());
        }
        rows.push(serde_json::json!({"id": input.id, "source_sha256": format!("{:x}", Sha256::digest(input.source.as_bytes())),
            "language": document.language, "result": result, "incomplete_rules": incomplete_rules, "section_annotations": seiso::sections::classify(&document)}));
    }
    rows.sort_by_key(|row| row["id"].as_str().unwrap().to_owned());
    fs::write(&args[2], serde_json::to_vec(&rows)?)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("replay: {error}");
        std::process::exit(2);
    }
}
