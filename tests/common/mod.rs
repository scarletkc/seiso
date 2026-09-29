#![allow(dead_code)]

use seiso::config::{CliOverrides, Config, ConfigError};
use seiso::md::Document;
use seiso::rules::{CheckResult, LocalWorkspaceFiles, RawCheckResult, WorkspaceFiles};
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

pub struct CheckContext<'a> {
    pub document: &'a Document,
    pub filename: &'a str,
    pub path: &'a Path,
    pub workspace_root: &'a Path,
    pub config: &'a Config,
    pub overrides: &'a CliOverrides,
}

impl CheckContext<'_> {
    pub fn check(&self, files: &dyn WorkspaceFiles) -> Result<RawCheckResult, ConfigError> {
        let context = seiso::rules::CheckContext::new(
            self.document,
            self.filename,
            self.path,
            self.workspace_root,
            self.config,
            self.overrides,
        )?;
        Ok(seiso::rules::check(&context, files))
    }
}

pub fn check(context: &CheckContext<'_>) -> Result<CheckResult, ConfigError> {
    Ok(context
        .check(&LocalWorkspaceFiles::default())?
        .finish(context.document, context.filename))
}

pub fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub fn run(root: &Path, arguments: &[&str], input: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_seiso"))
        .current_dir(root)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
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

pub fn value(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn workspace(config: &str) -> TempDir {
    let root = TempDir::new().unwrap();
    write(root.path(), "seiso.toml", config);
    root
}
