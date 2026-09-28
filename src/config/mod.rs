//! Configuration discovery, explicit inheritance, and per-file policy resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::paths::{SiteRoutes, normalize};
use crate::rules::{rule, rules};

pub const KINDS: [&str; 8] = [
    "readme",
    "howto",
    "reference",
    "runbook",
    "adr",
    "plan",
    "changelog",
    "generated",
];

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid configuration {path}: {message}")]
    Invalid { path: PathBuf, message: String },
    #[error("configuration inheritance cycle: {0}")]
    Cycle(String),
    #[error(
        "invalid rule selector {0:?} for seiso {version}; use ALL, a rule family such as KND, or a code listed by `seiso rule --all`; rules added in a newer release need an upgrade",
        version = env!("CARGO_PKG_VERSION")
    )]
    Selector(String),
    #[error("file {path} is outside workspace {root}")]
    OutsideWorkspace { path: PathBuf, root: PathBuf },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Settings {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub preview: bool,
    pub kinds: Vec<KindMapping>,
    pub domains: Vec<DomainMapping>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<SiteMapping>,
    pub lint: LintSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            include: vec!["**/*.md".into(), "**/*.markdown".into()],
            exclude: Vec::new(),
            preview: false,
            kinds: Vec::new(),
            domains: Vec::new(),
            sites: Vec::new(),
            lint: LintSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KindMapping {
    pub path: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainMapping {
    pub path: String,
    pub name: String,
}

/// Documents a site generator renders, and where their route links resolve.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SiteMapping {
    pub path: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<String>,
    #[serde(default = "default_base")]
    pub base: String,
}

fn default_base() -> String {
    "/".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct LintSettings {
    pub select: Vec<String>,
    pub ignore: Vec<String>,
    pub languages: Vec<String>,
    pub per_file_ignores: BTreeMap<String, Vec<String>>,
    pub dup: DupSettings,
    pub ptr: PtrSettings,
    pub lexicon: BTreeMap<String, Lexicon>,
}

impl Default for LintSettings {
    fn default() -> Self {
        Self {
            select: vec!["ALL".into()],
            ignore: Vec::new(),
            languages: vec!["en".into(), "zh".into(), "ja".into()],
            per_file_ignores: BTreeMap::new(),
            dup: DupSettings::default(),
            ptr: PtrSettings::default(),
            lexicon: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct DupSettings {
    pub min_identifiers: usize,
    pub min_jaccard: f64,
    pub min_paragraph_similarity: f64,
    pub min_paragraph_chars: usize,
    pub shingle_size: usize,
}

impl Default for DupSettings {
    fn default() -> Self {
        Self {
            min_identifiers: 5,
            min_jaccard: 0.8,
            min_paragraph_similarity: 0.9,
            min_paragraph_chars: 80,
            shingle_size: 5,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct PtrSettings {
    pub catalog_dirs: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Lexicon {
    pub extend_stale_markers: Vec<String>,
    pub extend_constraint_markers: Vec<String>,
    pub extend_commit_contexts: Vec<String>,
    pub extend_pointer_markers: Vec<String>,
    pub extend_source_pointers: Vec<String>,
    pub extend_rationale_headings: Vec<String>,
    pub extend_conversation_markers: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CliOverrides {
    pub select: Option<Vec<String>>,
    pub extend_select: Vec<String>,
    pub preview: bool,
}

impl CliOverrides {
    pub fn validate(&self) -> Result<(), ConfigError> {
        for selector in self.select.iter().flatten().chain(&self.extend_select) {
            validate_selector(selector)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub source: Option<PathBuf>,
    pub directory: PathBuf,
    pub settings: Settings,
    include: Vec<GlobMatcher>,
    exclude: Vec<GlobMatcher>,
    kinds: Vec<GlobMatcher>,
    domains: Vec<GlobMatcher>,
    sites: Vec<GlobMatcher>,
    per_file_ignores: Vec<(GlobMatcher, Vec<String>)>,
}

impl Config {
    /// Load a configuration and its explicit inheritance chain.
    ///
    /// Each `extend` path is relative to the file declaring it. All effective
    /// glob patterns, including inherited patterns, use the selected file's
    /// directory as their base.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let path = absolute(path)?;
        let value = load_extended(&path, &mut Vec::new())?;
        Self::from_value(value, parent(&path).to_path_buf(), Some(path))
    }

    /// Load an explicitly selected configuration whose patterns apply from `directory`.
    ///
    /// `extend` paths remain relative to the file declaring them.
    pub fn load_from(path: &Path, directory: &Path) -> Result<Self, ConfigError> {
        let path = absolute(path)?;
        let value = load_extended(&path, &mut Vec::new())?;
        Self::from_value(value, absolute(directory)?, Some(path))
    }

    /// Parse a standalone configuration; relative paths use `directory` as their base.
    pub fn parse(source: &str, directory: &Path) -> Result<Self, ConfigError> {
        let directory = absolute(directory)?;
        let path = directory.join("seiso.toml");
        let value = parse_toml(source, &path)?;
        if value.get("extend").is_some() {
            return Err(invalid(
                &path,
                "extend requires Config::load so its source file can be resolved",
            ));
        }
        Self::from_value(value, directory, None)
    }

    pub fn defaults(directory: &Path) -> Result<Self, ConfigError> {
        Self::parse("", directory)
    }

    fn from_value(
        value: toml::Value,
        directory: PathBuf,
        source: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let label = source
            .clone()
            .unwrap_or_else(|| directory.join("seiso.toml"));
        let settings: Settings = value.try_into().map_err(|e| invalid(&label, e))?;
        validate_settings(&settings, &label)?;
        let include = compile_patterns(&settings.include, &label, "include")?;
        let exclude = compile_patterns(&settings.exclude, &label, "exclude")?;
        let kinds = settings
            .kinds
            .iter()
            .map(|m| compile_pattern(&m.path, &label, "kinds.path"))
            .collect::<Result<_, _>>()?;
        let domains = settings
            .domains
            .iter()
            .map(|m| compile_pattern(&m.path, &label, "domains.path"))
            .collect::<Result<_, _>>()?;
        let sites = settings
            .sites
            .iter()
            .map(|m| compile_pattern(&m.path, &label, "sites.path"))
            .collect::<Result<_, _>>()?;
        let per_file_ignores = settings
            .lint
            .per_file_ignores
            .iter()
            .map(|(pattern, selectors)| {
                Ok((
                    compile_pattern(pattern, &label, "lint.per-file-ignores")?,
                    selectors.clone(),
                ))
            })
            .collect::<Result<_, ConfigError>>()?;
        Ok(Self {
            source,
            directory,
            settings,
            include,
            exclude,
            kinds,
            domains,
            sites,
            per_file_ignores,
        })
    }

    pub fn relative_path(&self, path: &Path) -> Option<String> {
        let path = normalize(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.directory.join(path)
        });
        path.strip_prefix(&self.directory)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    pub fn includes(&self, path: &Path) -> bool {
        self.relative_path(path)
            .is_some_and(|p| self.include.iter().any(|m| m.is_match(&p)))
    }

    pub fn excludes(&self, path: &Path) -> bool {
        self.relative_path(path)
            .is_some_and(|p| self.exclude.iter().any(|m| m.is_match(&p)))
    }

    pub fn kind_for(&self, path: &Path) -> Option<&str> {
        let path = self.relative_path(path)?;
        self.kinds
            .iter()
            .zip(&self.settings.kinds)
            .rev()
            .find(|(m, _)| m.is_match(&path))
            .map(|(_, entry)| entry.kind.as_str())
    }

    pub fn domain_for(&self, path: &Path) -> Option<&str> {
        let path = self.relative_path(path)?;
        self.domains
            .iter()
            .zip(&self.settings.domains)
            .rev()
            .find(|(m, _)| m.is_match(&path))
            .map(|(_, entry)| entry.name.as_str())
    }

    pub fn site_for(&self, path: &Path) -> Option<&SiteMapping> {
        if self.sites.is_empty() {
            return None;
        }
        let path = self.relative_path(path)?;
        self.sites
            .iter()
            .zip(&self.settings.sites)
            .rev()
            .find(|(m, _)| m.is_match(&path))
            .map(|(_, entry)| entry)
    }

    /// Resolve the matching site's directories from this configuration's directory.
    pub(crate) fn site_routes(&self, path: &Path) -> Option<SiteRoutes> {
        let site = self.site_for(path)?;
        let mut base = site.base.clone();
        if !base.ends_with('/') {
            base.push('/');
        }
        Some(SiteRoutes {
            root: normalize(self.directory.join(&site.root)),
            public: site
                .public
                .as_ref()
                .map(|public| normalize(self.directory.join(public))),
            base,
        })
    }

    /// Apply selection and applicability before exposing accepted or opt-in rules.
    pub fn enabled_rules(
        &self,
        path: &Path,
        kind: Option<&str>,
        overrides: &CliOverrides,
    ) -> Result<Vec<&'static str>, ConfigError> {
        Ok(self
            .selected_rules(path, overrides)?
            .into_iter()
            .filter(|code| rule(code).is_some_and(|rule| rule.applies_to_kind(kind)))
            .collect())
    }

    /// Resolve selectors before parsing a document so callers can plan its dependencies.
    pub fn selected_rules(
        &self,
        path: &Path,
        overrides: &CliOverrides,
    ) -> Result<Vec<&'static str>, ConfigError> {
        overrides.validate()?;
        let select = overrides
            .select
            .as_ref()
            .unwrap_or(&self.settings.lint.select);
        let selectors: Vec<_> = select.iter().chain(&overrides.extend_select).collect();
        let relative_path = self.relative_path(path);
        let mut enabled: Vec<_> = rules()
            .iter()
            .filter(|rule| self.settings.preview || overrides.preview || rule.is_stable())
            .map(|rule| rule.code)
            .filter(|code| {
                let selected = selectors.iter().filter_map(|s| specificity(s, code)).max();
                let ignored = self
                    .settings
                    .lint
                    .ignore
                    .iter()
                    .filter_map(|s| specificity(s, code))
                    .max();
                let selected = selected.is_some_and(|s| ignored.is_none_or(|i| s > i));
                selected
                    && !self.per_file_ignores.iter().any(|(pattern, entries)| {
                        relative_path.as_ref().is_some_and(|p| pattern.is_match(p))
                            && entries.iter().any(|s| specificity(s, code).is_some())
                    })
            })
            .collect();
        enabled.sort_unstable();
        Ok(enabled)
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: Config,
    explicit_config: bool,
}

impl Workspace {
    pub fn discover(cwd: &Path, explicit_config: Option<&Path>) -> Result<Self, ConfigError> {
        let cwd = absolute(cwd)?;
        if let Some(path) = explicit_config {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            // An explicit configuration selects policy, not the workspace: its
            // patterns apply from the repository root around the caller.
            let root = repository_root(&cwd);
            let config = Config::load_from(&path, &root)?;
            return Ok(Self {
                root,
                config,
                explicit_config: true,
            });
        }
        for directory in cwd.ancestors() {
            if let Some(path) = config_in(directory)? {
                let config = Config::load(&path)?;
                return Ok(Self {
                    root: directory.to_path_buf(),
                    config,
                    explicit_config: false,
                });
            }
        }
        let root = repository_root(&cwd);
        let config = Config::defaults(&root)?;
        Ok(Self {
            root,
            config,
            explicit_config: false,
        })
    }

    pub fn config_for(&self, file: &Path) -> Result<Config, ConfigError> {
        let file = normalize(if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.root.join(file)
        });
        if !file.starts_with(&self.root) {
            return Err(ConfigError::OutsideWorkspace {
                path: file,
                root: self.root.clone(),
            });
        }
        if self.explicit_config {
            return Ok(self.config.clone());
        }
        for directory in parent(&file)
            .ancestors()
            .take_while(|d| d.starts_with(&self.root))
        {
            if directory == self.root {
                return Ok(self.config.clone());
            }
            if let Some(path) = config_in(directory)? {
                return Config::load(&path);
            }
        }
        Ok(self.config.clone())
    }
}

fn repository_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

pub fn validate_selector(selector: &str) -> Result<(), ConfigError> {
    if selector == "ALL"
        || rule(selector).is_some()
        || (selector.len() == 3 && rules().iter().any(|rule| rule.code.starts_with(selector)))
    {
        Ok(())
    } else {
        Err(ConfigError::Selector(selector.to_owned()))
    }
}

fn specificity(selector: &str, code: &str) -> Option<u8> {
    if selector == code {
        Some(2)
    } else if code.starts_with(selector) {
        Some(1)
    } else if selector == "ALL" {
        Some(0)
    } else {
        None
    }
}

fn validate_settings(settings: &Settings, path: &Path) -> Result<(), ConfigError> {
    for mapping in &settings.kinds {
        if !KINDS.contains(&mapping.kind.as_str()) {
            return Err(invalid(
                path,
                format!(
                    "unknown kind {:?}; expected {}",
                    mapping.kind,
                    KINDS.join(", ")
                ),
            ));
        }
    }
    for mapping in &settings.domains {
        if mapping.name.trim().is_empty() {
            return Err(invalid(path, "domains.name must not be empty"));
        }
    }
    for site in &settings.sites {
        for (field, value) in [
            ("sites.root", Some(&site.root)),
            ("sites.public", site.public.as_ref()),
        ] {
            if let Some(value) = value
                && (value.trim().is_empty()
                    || value.starts_with(['/', '\\'])
                    || Path::new(value).is_absolute())
            {
                return Err(invalid(
                    path,
                    format!(
                        "{field} {value:?} must be a directory relative to this configuration, such as \"docs\""
                    ),
                ));
            }
        }
        if !site.base.starts_with('/') || site.base.contains(['?', '#']) {
            return Err(invalid(
                path,
                format!(
                    "sites.base {:?} must be a URL path that starts with /, such as \"/guide/\"",
                    site.base
                ),
            ));
        }
    }
    for selector in settings
        .lint
        .select
        .iter()
        .chain(&settings.lint.ignore)
        .chain(settings.lint.per_file_ignores.values().flatten())
    {
        validate_selector(selector).map_err(|e| invalid(path, e))?;
    }
    for lang in settings
        .lint
        .languages
        .iter()
        .chain(settings.lint.lexicon.keys())
    {
        if !["en", "zh", "ja"].contains(&lang.as_str()) {
            return Err(invalid(
                path,
                format!("unsupported language {lang:?}; expected en, zh, or ja"),
            ));
        }
    }
    if settings.lint.dup.min_identifiers == 0 {
        return Err(invalid(path, "lint.dup.min-identifiers must be at least 1"));
    }
    if !settings.lint.dup.min_jaccard.is_finite()
        || !(0.0..=1.0).contains(&settings.lint.dup.min_jaccard)
    {
        return Err(invalid(
            path,
            "lint.dup.min-jaccard must be a finite number from 0 to 1",
        ));
    }
    if !settings.lint.dup.min_paragraph_similarity.is_finite()
        || settings.lint.dup.min_paragraph_similarity <= 0.0
        || settings.lint.dup.min_paragraph_similarity > 1.0
    {
        return Err(invalid(
            path,
            "lint.dup.min-paragraph-similarity must be greater than 0 and at most 1",
        ));
    }
    if settings.lint.dup.min_paragraph_chars == 0 || settings.lint.dup.shingle_size == 0 {
        return Err(invalid(
            path,
            "lint.dup.min-paragraph-chars and shingle-size must be positive",
        ));
    }
    for dir in &settings.lint.ptr.catalog_dirs {
        if dir.trim().is_empty() {
            return Err(invalid(
                path,
                "lint.ptr.catalog-dirs entries must not be empty",
            ));
        }
    }
    for lexicon in settings.lint.lexicon.values() {
        for (name, entries) in [
            ("extend-stale-markers", &lexicon.extend_stale_markers),
            (
                "extend-constraint-markers",
                &lexicon.extend_constraint_markers,
            ),
            ("extend-commit-contexts", &lexicon.extend_commit_contexts),
            ("extend-pointer-markers", &lexicon.extend_pointer_markers),
            ("extend-source-pointers", &lexicon.extend_source_pointers),
            (
                "extend-rationale-headings",
                &lexicon.extend_rationale_headings,
            ),
            (
                "extend-conversation-markers",
                &lexicon.extend_conversation_markers,
            ),
        ] {
            if entries.iter().any(|entry| entry.trim().is_empty()) {
                return Err(invalid(
                    path,
                    format!("lint.lexicon {name} entries must not be empty"),
                ));
            }
        }
    }
    Ok(())
}

fn compile_patterns(
    patterns: &[String],
    path: &Path,
    field: &str,
) -> Result<Vec<GlobMatcher>, ConfigError> {
    patterns
        .iter()
        .map(|p| compile_pattern(p, path, field))
        .collect()
}

fn compile_pattern(pattern: &str, path: &Path, field: &str) -> Result<GlobMatcher, ConfigError> {
    if pattern.is_empty() {
        return Err(invalid(path, format!("{field} glob must not be empty")));
    }
    GlobBuilder::new(&pattern.replace('\\', "/"))
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| invalid(path, format!("invalid {field} glob {pattern:?}: {e}")))
}

fn config_in(directory: &Path) -> Result<Option<PathBuf>, ConfigError> {
    for name in [".seiso.toml", "seiso.toml", "pyproject.toml"] {
        let path = directory.join(name);
        match fs::metadata(&path) {
            Ok(meta) if meta.is_file() => {
                if name != "pyproject.toml" || read_document(&path)?.is_some() {
                    return Ok(Some(path));
                }
            }
            Ok(_) => {
                return Err(invalid(
                    &path,
                    "expected a configuration file, found a directory",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(ConfigError::Read { path, source }),
        }
    }
    Ok(None)
}

fn load_extended(path: &Path, stack: &mut Vec<PathBuf>) -> Result<toml::Value, ConfigError> {
    if stack.len() >= 128 {
        return Err(invalid(
            path,
            "configuration inheritance exceeds 128 files; shorten the extend chain",
        ));
    }
    let identity = fs::canonicalize(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if stack.contains(&identity) {
        let chain = stack
            .iter()
            .chain(std::iter::once(&identity))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(ConfigError::Cycle(chain));
    }
    stack.push(identity);
    let mut value = read_document(path)?
        .ok_or_else(|| invalid(path, "pyproject.toml has no [tool.seiso] table"))?;
    let extension = value
        .as_table_mut()
        .and_then(|table| table.remove("extend"));
    if let Some(extension) = extension {
        let extension = extension
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(path, "extend must be a non-empty configuration file path"))?;
        let inherited_path = normalize(parent(path).join(extension));
        let mut base = load_extended(&inherited_path, stack)?;
        overlay(&mut base, value);
        value = base;
    }
    stack.pop();
    Ok(value)
}

fn overlay(base: &mut toml::Value, local: toml::Value) {
    match (base, local) {
        (toml::Value::Table(base), toml::Value::Table(local)) => {
            for (key, value) in local {
                if let Some(existing) = base.get_mut(&key) {
                    overlay(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, local) => *base = local,
    }
}

fn read_document(path: &Path) -> Result<Option<toml::Value>, ConfigError> {
    let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = parse_toml(&source, path)?;
    if path
        .file_name()
        .is_some_and(|name| name == "pyproject.toml")
    {
        Ok(value.get("tool").and_then(|t| t.get("seiso")).cloned())
    } else {
        Ok(Some(value))
    }
}

fn parse_toml(source: &str, path: &Path) -> Result<toml::Value, ConfigError> {
    toml::from_str(source).map_err(|e| invalid(path, e))
}

fn invalid(path: &Path, message: impl std::fmt::Display) -> ConfigError {
    ConfigError::Invalid {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(path)
}

fn absolute(path: &Path) -> Result<PathBuf, ConfigError> {
    std::path::absolute(path)
        .map(normalize)
        .map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })
}
