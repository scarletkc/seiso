//! Origin directories for path entries, kept separately from user settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Origins mirror list replacement and per-file-ignore table merging.
#[derive(Clone, Debug, Default)]
pub(super) struct PatternBases {
    pub lists: BTreeMap<String, Vec<PathBuf>>,
    pub ignores: BTreeMap<String, PathBuf>,
}

impl PatternBases {
    /// Record the effective directory of entries declared in one configuration.
    pub fn new(value: &toml::Value, directory: &Path) -> Self {
        let mut result = Self::default();
        for field in [
            "include",
            "exclude",
            "extend-exclude",
            "kinds",
            "domains",
            "sites",
        ] {
            if let Some(entries) = value.get(field).and_then(toml::Value::as_array) {
                result
                    .lists
                    .insert(field.into(), vec![directory.to_owned(); entries.len()]);
            }
        }
        if let Some(lint) = value.get("lint") {
            if let Some(entries) = lint.get("per-file-ignores").and_then(toml::Value::as_table) {
                result.ignores = entries
                    .keys()
                    .map(|key| (key.clone(), directory.to_owned()))
                    .collect();
            }
            if let Some(entries) = lint
                .get("ptr")
                .and_then(|ptr| ptr.get("catalog-dirs"))
                .and_then(toml::Value::as_array)
            {
                result.lists.insert(
                    "lint.ptr.catalog-dirs".into(),
                    vec![directory.to_owned(); entries.len()],
                );
            }
        }
        result
    }

    /// Lists replace inherited origins; table entries override only matching keys.
    pub fn overlay(&mut self, local: Self) {
        for (field, entries) in local.lists {
            if field.starts_with("extend-") {
                self.lists.entry(field).or_default().extend(entries);
            } else {
                self.lists.insert(field, entries);
            }
        }
        self.ignores.extend(local.ignores);
    }

    pub fn apply_extensions(&mut self) {
        if let Some(entries) = self.lists.remove("extend-exclude") {
            self.lists
                .entry("exclude".into())
                .or_default()
                .extend(entries);
        }
    }

    /// Defaults have no declaration and use the selected configuration's directory.
    pub fn directory<'a>(&'a self, field: &str, index: usize, default: &'a Path) -> &'a Path {
        self.lists
            .get(field)
            .and_then(|entries| entries.get(index))
            .map(PathBuf::as_path)
            .unwrap_or(default)
    }
}

/// A path entry and the directory from which its pattern or path is interpreted.
#[derive(Clone, Debug, Serialize)]
pub struct PatternBase {
    pub pattern: String,
    pub base_directory: String,
}

/// Render origins relative to the inspected workspace, including ancestor bases.
pub(super) fn relative_directory(root: &Path, directory: &Path) -> String {
    for ancestor in root.ancestors() {
        if let Ok(suffix) = directory.strip_prefix(ancestor) {
            let mut relative = PathBuf::new();
            for _ in root.strip_prefix(ancestor).unwrap().components() {
                relative.push("..");
            }
            if !suffix.as_os_str().is_empty() {
                relative.push(suffix);
            }
            return if relative.as_os_str().is_empty() {
                ".".into()
            } else {
                relative.to_string_lossy().replace('\\', "/")
            };
        }
    }
    directory.to_string_lossy().replace('\\', "/")
}
