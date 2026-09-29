//! Configuration discovery, explicit inheritance, and per-file policy resolution.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::paths::{SiteRoutes, normalize};
use crate::rules::{rule, rules};

mod frame;

use frame::{EnvironmentFrame, FrameId, FrameRelation, FramedKind, FramedValue, overlay};

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
    frames: Vec<EnvironmentFrame>,
    include: Vec<BoundGlob>,
    exclude: Vec<BoundGlob>,
    kinds: Vec<BoundGlob>,
    domains: Vec<BoundGlob>,
    sites: Vec<BoundGlob>,
    per_file_ignores: Vec<(BoundGlob, Vec<String>)>,
    catalog_dirs: Vec<PathBuf>,
    catalog_frames: Vec<FrameId>,
}

/// The declaration environments and effective path-field bindings of a policy.
///
/// Binding arrays align by index with the corresponding arrays in `Settings`.
/// This additive report leaves the established settings JSON shape unchanged.
#[derive(Clone, Debug, Serialize)]
pub struct EnvironmentReport {
    pub frames: Vec<FrameReport>,
    pub include: Vec<FrameId>,
    pub exclude: Vec<FrameId>,
    pub kinds: Vec<FrameId>,
    pub domains: Vec<FrameId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<FrameId>,
    pub per_file_ignores: BTreeMap<String, FrameId>,
    pub catalog_dirs: Vec<FrameId>,
}

/// A configuration unit's source, path base, and incoming inheritance edge.
#[derive(Clone, Debug, Serialize)]
pub struct FrameReport {
    pub source: String,
    pub policy_base: String,
    pub extender: Option<FrameId>,
    pub relation: &'static str,
}

/// A compiled pattern bound to the environment that declared it.
#[derive(Clone, Debug)]
struct BoundGlob {
    matcher: GlobMatcher,
    frame: FrameId,
}

/// Candidate paths cached by effective base while matching one file.
///
/// Most policies use one base, so the first entry avoids tree lookups. The
/// overflow map preserves bounded lookup cost for deeply mixed frame chains.
#[derive(Default)]
struct CandidatePaths<'a> {
    first: Option<(&'a Path, Option<String>)>,
    overflow: BTreeMap<&'a Path, Option<String>>,
}

impl<'a> CandidatePaths<'a> {
    /// Return the file path relative to this base, computing it only once.
    fn relative(&mut self, base: &'a Path, path: &Path) -> Option<&str> {
        if self.first.is_none() {
            self.first = Some((base, relative_to(path, base)));
        }
        let (first_base, first_path) = self.first.as_ref().unwrap();
        if std::ptr::eq(*first_base, base) || *first_base == base {
            return first_path.as_deref();
        }
        self.overflow
            .entry(base)
            .or_insert_with(|| relative_to(path, base))
            .as_deref()
    }
}

/// Normalize a candidate once for all patterns bound to the same base.
fn relative_to(path: &Path, base: &Path) -> Option<String> {
    path.strip_prefix(base)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

impl Config {
    /// The outermost governing ancestor admitted by this configuration chain.
    ///
    /// Shared templates do not widen a project's document universe merely
    /// because their source files live elsewhere. A governing frame does.
    pub fn project_root(&self, selection_root: &Path) -> PathBuf {
        let mut root = selection_root.to_path_buf();
        for frame in &self.frames {
            if frame.relation == FrameRelation::GoverningAncestor
                && root.starts_with(&frame.policy_base)
            {
                root = frame.policy_base.clone();
            }
        }
        root
    }

    /// Root admitted by this selected configuration's own governing chain.
    ///
    /// A wider index may serve another file whose configuration inherits a
    /// parent, but that must not grant this configuration the same scope.
    pub fn project_root_for_source(&self) -> PathBuf {
        self.project_root(&self.directory)
    }

    /// Include the project's scope already granted by the invoking policy.
    ///
    /// A standalone nested config must not shrink the historical workspace
    /// visible when seiso was invoked from a wider root. Conversely, a nested
    /// governing ancestor may expand a narrower invocation's scope.
    pub fn project_root_for_source_within(&self, invocation_root: &Path) -> PathBuf {
        let own_root = self.project_root_for_source();
        if invocation_root.starts_with(&own_root) {
            own_root
        } else if own_root.starts_with(invocation_root) {
            invocation_root.to_path_buf()
        } else {
            // A dependency in a sibling subtree was not governed by the
            // invoking configuration. Keep its own root, not the filesystem
            // common ancestor of two independently admitted scopes.
            own_root
        }
    }

    /// Load a configuration and its explicit inheritance chain.
    ///
    /// Each `extend` path is relative to the declaring file. A governing
    /// ancestor retains its own path base; a shared template inherits the
    /// extending unit's base.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let path = absolute(path)?;
        let directory = parent(&path).to_path_buf();
        let mut frames = vec![EnvironmentFrame {
            source: path.clone(),
            policy_base: directory.clone(),
            extender: None,
            relation: FrameRelation::Selected,
        }];
        let value = load_extended(&path, 0, &mut frames, &mut Vec::new())?;
        Self::from_value(value, frames, directory, Some(path))
    }

    /// Load an explicitly selected configuration whose patterns apply from `directory`.
    ///
    /// `extend` paths remain relative to the file declaring them.
    pub fn load_from(path: &Path, directory: &Path) -> Result<Self, ConfigError> {
        let path = absolute(path)?;
        let directory = absolute(directory)?;
        let mut frames = vec![EnvironmentFrame {
            source: path.clone(),
            policy_base: directory.clone(),
            extender: None,
            relation: FrameRelation::Selected,
        }];
        let value = load_extended(&path, 0, &mut frames, &mut Vec::new())?;
        Self::from_value(value, frames, directory, Some(path))
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
        let frames = vec![EnvironmentFrame {
            source: path,
            policy_base: directory.clone(),
            extender: None,
            relation: FrameRelation::Selected,
        }];
        Self::from_value(FramedValue::bind(value, 0), frames, directory, None)
    }

    pub fn defaults(directory: &Path) -> Result<Self, ConfigError> {
        Self::parse("", directory)
    }

    fn from_value(
        value: FramedValue,
        frames: Vec<EnvironmentFrame>,
        directory: PathBuf,
        source: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let label = source
            .clone()
            .unwrap_or_else(|| directory.join("seiso.toml"));
        let mut value = value;
        let include_frames = entry_frames(value.get("include"), 0);
        let mut exclude_frames = entry_frames(value.get("exclude"), 0);
        let mut kind_frames = entry_frames(value.get("kinds"), 0);
        let mut domain_frames = entry_frames(value.get("domains"), 0);
        let mut site_frames = entry_frames(value.get("sites"), 0);
        let catalog_frames = entry_frames(
            value
                .get("lint")
                .and_then(|lint| lint.get("ptr"))
                .and_then(|ptr| ptr.get("catalog-dirs")),
            0,
        );
        let per_file_frames: BTreeMap<_, _> = value
            .get("lint")
            .and_then(|lint| lint.get("per-file-ignores"))
            .and_then(FramedValue::table)
            .into_iter()
            .flat_map(|table| table.iter().map(|(key, value)| (key.clone(), value.frame)))
            .collect();

        let extend_exclude = take_string_extension_list(&mut value, "extend-exclude", &label)?;
        let extend_kinds =
            take_mapping_extension_list::<KindMapping>(&mut value, "extend-kinds", &label)?;
        let extend_domains =
            take_mapping_extension_list::<DomainMapping>(&mut value, "extend-domains", &label)?;
        let extend_sites =
            take_mapping_extension_list::<SiteMapping>(&mut value, "extend-sites", &label)?;
        let (extend_select, extend_ignore) = if let Some(lint) = value.get_mut("lint") {
            (
                take_string_extension_list(lint, "extend-select", &label)?,
                take_string_extension_list(lint, "extend-ignore", &label)?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let mut settings: Settings = value
            .into_toml()
            .try_into()
            .map_err(|e| invalid(&label, e))?;
        settings
            .exclude
            .extend(extend_exclude.iter().map(|(entry, _)| entry.clone()));
        exclude_frames.extend(extend_exclude.into_iter().map(|(_, frame)| frame));
        settings
            .kinds
            .extend(extend_kinds.iter().map(|(entry, _)| entry.clone()));
        kind_frames.extend(extend_kinds.into_iter().map(|(_, frame)| frame));
        settings
            .domains
            .extend(extend_domains.iter().map(|(entry, _)| entry.clone()));
        domain_frames.extend(extend_domains.into_iter().map(|(_, frame)| frame));
        settings
            .sites
            .extend(extend_sites.iter().map(|(entry, _)| entry.clone()));
        site_frames.extend(extend_sites.into_iter().map(|(_, frame)| frame));
        settings
            .lint
            .select
            .extend(extend_select.into_iter().map(|(entry, _)| entry));
        settings
            .lint
            .ignore
            .extend(extend_ignore.into_iter().map(|(entry, _)| entry));
        validate_settings(&settings, &label)?;
        let include = compile_bound(
            &settings.include,
            &include_frames,
            &frames,
            &label,
            "include",
        )?;
        let exclude = compile_bound(
            &settings.exclude,
            &exclude_frames,
            &frames,
            &label,
            "exclude",
        )?;
        let kinds = compile_bound(
            &settings
                .kinds
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            &kind_frames,
            &frames,
            &label,
            "kinds.path",
        )?;
        let domains = compile_bound(
            &settings
                .domains
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            &domain_frames,
            &frames,
            &label,
            "domains.path",
        )?;
        let sites = compile_bound(
            &settings
                .sites
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            &site_frames,
            &frames,
            &label,
            "sites.path",
        )?;
        let per_file_ignores = settings
            .lint
            .per_file_ignores
            .iter()
            .map(|(pattern, selectors)| {
                let frame = *per_file_frames.get(pattern).unwrap_or(&0);
                Ok((
                    BoundGlob {
                        matcher: compile_pattern(
                            pattern,
                            &frames[frame].source,
                            "lint.per-file-ignores",
                        )?,
                        frame,
                    },
                    selectors.clone(),
                ))
            })
            .collect::<Result<_, ConfigError>>()?;
        let catalog_dirs = settings
            .lint
            .ptr
            .catalog_dirs
            .iter()
            .enumerate()
            .map(|(i, dir)| {
                let frame = *catalog_frames.get(i).unwrap_or(&0);
                normalize(frames[frame].policy_base.join(dir.trim_end_matches('/')))
            })
            .collect();
        let catalog_frames = (0..settings.lint.ptr.catalog_dirs.len())
            .map(|index| *catalog_frames.get(index).unwrap_or(&0))
            .collect();
        Ok(Self {
            source,
            directory,
            settings,
            frames,
            include,
            exclude,
            kinds,
            domains,
            sites,
            per_file_ignores,
            catalog_dirs,
            catalog_frames,
        })
    }

    /// Describe the environment of every effective path-bearing declaration.
    pub fn environment_report(&self, root: &Path) -> EnvironmentReport {
        let frames = self
            .frames
            .iter()
            .map(|frame| FrameReport {
                source: relative_label(root, &frame.source),
                policy_base: relative_label(root, &frame.policy_base),
                extender: frame.extender,
                relation: match frame.relation {
                    FrameRelation::Selected => "selected",
                    FrameRelation::GoverningAncestor => "governing-ancestor",
                    FrameRelation::SharedTemplate => "shared-template",
                },
            })
            .collect();
        EnvironmentReport {
            frames,
            include: self.include.iter().map(|entry| entry.frame).collect(),
            exclude: self.exclude.iter().map(|entry| entry.frame).collect(),
            kinds: self.kinds.iter().map(|entry| entry.frame).collect(),
            domains: self.domains.iter().map(|entry| entry.frame).collect(),
            sites: self.sites.iter().map(|entry| entry.frame).collect(),
            per_file_ignores: self
                .settings
                .lint
                .per_file_ignores
                .keys()
                .zip(&self.per_file_ignores)
                .map(|(key, (entry, _))| (key.clone(), entry.frame))
                .collect(),
            catalog_dirs: self.catalog_frames.clone(),
        }
    }

    pub fn relative_path(&self, path: &Path) -> Option<String> {
        let path = self.absolute_path(path);
        path.strip_prefix(&self.directory)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    fn absolute_path(&self, path: &Path) -> PathBuf {
        normalize(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.directory.join(path)
        })
    }

    /// Compute each distinct frame base's candidate path at most once.
    fn matches<'a>(
        &'a self,
        pattern: &BoundGlob,
        path: &Path,
        paths: &mut CandidatePaths<'a>,
    ) -> bool {
        let base = self.frames[pattern.frame].policy_base.as_path();
        paths
            .relative(base, path)
            .is_some_and(|relative| pattern.matcher.is_match(relative))
    }

    /// Find a matching entry without losing declaration-order precedence.
    ///
    /// A standalone config has one environment: derive its candidate once,
    /// then run matchers directly. Mixed environments use per-base reuse.
    fn matching_index(&self, path: &Path, patterns: &[BoundGlob], last: bool) -> Option<usize> {
        if patterns.is_empty() {
            return None;
        }
        let path = self.absolute_path(path);
        if self.frames.len() == 1 {
            let relative = relative_to(&path, &self.frames[0].policy_base)?;
            return if last {
                patterns
                    .iter()
                    .rposition(|pattern| pattern.matcher.is_match(&relative))
            } else {
                patterns
                    .iter()
                    .position(|pattern| pattern.matcher.is_match(&relative))
            };
        }
        let mut paths = CandidatePaths::default();
        if last {
            patterns
                .iter()
                .rposition(|pattern| self.matches(pattern, &path, &mut paths))
        } else {
            patterns
                .iter()
                .position(|pattern| self.matches(pattern, &path, &mut paths))
        }
    }

    pub fn includes(&self, path: &Path) -> bool {
        self.matching_index(path, &self.include, false).is_some()
    }

    pub fn excludes(&self, path: &Path) -> bool {
        self.matching_index(path, &self.exclude, false).is_some()
    }

    pub fn kind_for(&self, path: &Path) -> Option<&str> {
        self.matching_index(path, &self.kinds, true)
            .map(|index| self.settings.kinds[index].kind.as_str())
    }

    pub fn domain_for(&self, path: &Path) -> Option<&str> {
        self.matching_index(path, &self.domains, true)
            .map(|index| self.settings.domains[index].name.as_str())
    }

    fn site_index(&self, path: &Path) -> Option<usize> {
        self.matching_index(path, &self.sites, true)
    }

    pub fn site_for(&self, path: &Path) -> Option<&SiteMapping> {
        self.site_index(path)
            .map(|index| &self.settings.sites[index])
    }

    /// Reject site directories outside this source's admitted project root.
    ///
    /// The caller derives `root` from the invocation and this configuration's
    /// governing frames before validation; an inherited parent is ordinary
    /// project scope here, not an exception to containment.
    fn with_sites_inside(self, root: &Path) -> Result<Self, ConfigError> {
        let label = self
            .source
            .clone()
            .unwrap_or_else(|| self.directory.join("seiso.toml"));
        for (site, pattern) in self.settings.sites.iter().zip(&self.sites) {
            let base = &self.frames[pattern.frame].policy_base;
            for (field, value) in [
                ("sites.root", Some(&site.root)),
                ("sites.public", site.public.as_ref()),
            ] {
                if let Some(value) = value {
                    let resolved = normalize(base.join(value));
                    if !resolved.starts_with(root) {
                        return Err(invalid(
                            &label,
                            format!(
                                "{field} {value:?} is outside workspace {}; choose a directory inside the workspace",
                                root.display()
                            ),
                        ));
                    }
                }
            }
        }
        Ok(self)
    }

    /// Resolve the matching site's directories in its declaration frame.
    pub(crate) fn site_routes(&self, path: &Path) -> Option<SiteRoutes> {
        let index = self.site_index(path)?;
        let site = &self.settings.sites[index];
        let directory = &self.frames[self.sites[index].frame].policy_base;
        let mut base = site.base.clone();
        if !base.ends_with('/') {
            base.push('/');
        }
        Some(SiteRoutes {
            root: normalize(directory.join(&site.root)),
            public: site
                .public
                .as_ref()
                .map(|public| normalize(directory.join(public))),
            base,
        })
    }

    /// Whether a resolved path is a configured pointer catalog directory.
    pub fn is_catalog_dir(&self, path: &Path) -> bool {
        self.catalog_dirs.iter().any(|directory| directory == path)
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
        let path = self.absolute_path(path);
        let mut paths = CandidatePaths::default();
        let per_file_ignores: Vec<_> = self
            .per_file_ignores
            .iter()
            .filter(|(pattern, _)| self.matches(pattern, &path, &mut paths))
            .flat_map(|(_, entries)| entries.iter())
            .collect();
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
                    && !per_file_ignores
                        .iter()
                        .any(|selector| specificity(selector, code).is_some())
            })
            .collect();
        enabled.sort_unstable();
        Ok(enabled)
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    /// Default selection and user-facing path base for this invocation.
    pub root: PathBuf,
    /// Containment and index base admitted by governing ancestor frames.
    pub project_root: PathBuf,
    pub config: Config,
    explicit_config: bool,
    resolver: Arc<Mutex<ConfigResolver>>,
}

/// Invocation-scoped discovery and compiled-policy cache.
///
/// A selected config path uniquely determines its normal-discovery base.
/// Shared templates are elaborated within that selected config, never cached
/// by template path alone because the same template can have different bases.
#[derive(Debug, Default)]
struct ConfigResolver {
    directories: BTreeMap<PathBuf, Option<PathBuf>>,
    compiled: BTreeMap<PathBuf, Config>,
}

impl ConfigResolver {
    /// Return the selected nested source, or the workspace's own configuration.
    fn selected(&mut self, directory: &Path, root: &Path) -> Result<Option<PathBuf>, ConfigError> {
        let mut visited = Vec::new();
        let mut selected = None;
        for ancestor in directory
            .ancestors()
            .take_while(|path| path.starts_with(root))
        {
            if let Some(cached) = self.directories.get(ancestor) {
                selected = cached.clone();
                break;
            }
            visited.push(ancestor.to_path_buf());
            if let Some(path) = config_in(ancestor)? {
                selected = Some(path);
                break;
            }
            if ancestor == root {
                break;
            }
        }
        for directory in visited {
            self.directories.insert(directory, selected.clone());
        }
        Ok(selected)
    }
}

impl Workspace {
    fn new(root: PathBuf, project_root: PathBuf, config: Config, explicit_config: bool) -> Self {
        Self {
            root,
            project_root,
            config,
            explicit_config,
            resolver: Arc::new(Mutex::new(ConfigResolver::default())),
        }
    }

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
            let config = Config::load_from(&path, &root)?.with_sites_inside(&root)?;
            return Ok(Self::new(root.clone(), root, config, true));
        }
        for directory in cwd.ancestors() {
            if let Some(path) = config_in(directory)? {
                let config = Config::load(&path)?;
                let project_root = config.project_root(directory);
                let config = config.with_sites_inside(&project_root)?;
                return Ok(Self::new(
                    directory.to_path_buf(),
                    project_root,
                    config,
                    false,
                ));
            }
        }
        let root = repository_root(&cwd);
        let config = Config::defaults(&root)?;
        Ok(Self::new(root.clone(), root, config, false))
    }

    pub fn config_for(&self, file: &Path) -> Result<Config, ConfigError> {
        self.config_for_within(file, &self.project_root)
    }

    /// Resolve policy inside an admitted project root discovered during the
    /// selected-file preflight, without changing the invocation's scan root.
    pub(crate) fn config_for_within(
        &self,
        file: &Path,
        project_root: &Path,
    ) -> Result<Config, ConfigError> {
        let file = normalize(if file.is_absolute() {
            file.to_path_buf()
        } else {
            self.root.join(file)
        });
        if !file.starts_with(project_root) {
            return Err(ConfigError::OutsideWorkspace {
                path: file,
                root: project_root.to_path_buf(),
            });
        }
        if self.explicit_config {
            return Ok(self.config.clone());
        }
        let mut resolver = self
            .resolver
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(path) = resolver.selected(parent(&file), project_root)? else {
            return Config::defaults(project_root);
        };
        if self.config.source.as_ref() == Some(&path) {
            return Ok(self.config.clone());
        }
        if let Some(config) = resolver.compiled.get(&path) {
            return Ok(config.clone());
        }
        let config = Config::load(&path)?;
        let allowed_root = config.project_root_for_source_within(&self.project_root);
        let config = config.with_sites_inside(&allowed_root)?;
        resolver.compiled.insert(path, config.clone());
        Ok(config)
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

/// Compile effective patterns with their surviving declaration environments.
fn compile_bound(
    patterns: &[String],
    origins: &[FrameId],
    frames: &[EnvironmentFrame],
    path: &Path,
    field: &str,
) -> Result<Vec<BoundGlob>, ConfigError> {
    patterns
        .iter()
        .enumerate()
        .map(|(index, pattern)| {
            let frame = *origins.get(index).unwrap_or(&0);
            Ok(BoundGlob {
                matcher: compile_pattern(
                    pattern,
                    frames.get(frame).map_or(path, |f| &f.source),
                    field,
                )?,
                frame,
            })
        })
        .collect()
}

/// Frame IDs of an effective array; absent arrays inherit default settings.
fn entry_frames(value: Option<&FramedValue>, _default: FrameId) -> Vec<FrameId> {
    value
        .and_then(FramedValue::array)
        .map(|entries| entries.iter().map(|entry| entry.frame).collect())
        .unwrap_or_default()
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

fn load_extended(
    path: &Path,
    frame: FrameId,
    frames: &mut Vec<EnvironmentFrame>,
    stack: &mut Vec<PathBuf>,
) -> Result<FramedValue, ConfigError> {
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
    let mut value = FramedValue::bind(
        read_document(path)?
            .ok_or_else(|| invalid(path, "pyproject.toml has no [tool.seiso] table"))?,
        frame,
    );
    let extension = value.remove("extend");
    if let Some(extension) = extension {
        let extension = extension
            .string()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(path, "extend must be a non-empty configuration file path"))?;
        let inherited_path = normalize(parent(path).join(extension));
        let relation = if governing_ancestor(path, &inherited_path)? {
            FrameRelation::GoverningAncestor
        } else {
            FrameRelation::SharedTemplate
        };
        let policy_base = if relation == FrameRelation::GoverningAncestor {
            parent(&inherited_path).to_path_buf()
        } else {
            frames[frame].policy_base.clone()
        };
        let next_frame = frames.len();
        frames.push(EnvironmentFrame {
            source: inherited_path.clone(),
            policy_base,
            extender: Some(frame),
            relation,
        });
        let mut base = load_extended(&inherited_path, next_frame, frames, stack)?;
        overlay(&mut base, value);
        value = base;
    }
    stack.pop();
    Ok(value)
}

/// A governing edge is selected by ordinary discovery in a proper ancestor.
fn governing_ancestor(path: &Path, inherited: &Path) -> Result<bool, ConfigError> {
    let source_dir = parent(path);
    let target_dir = parent(inherited);
    if target_dir == source_dir || !source_dir.starts_with(target_dir) {
        return Ok(false);
    }
    let name = inherited.file_name().and_then(|name| name.to_str());
    let named = |expected: &str| {
        name.is_some_and(|name| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(expected)
            } else {
                name == expected
            }
        })
    };
    if named(".seiso.toml") {
        return Ok(true);
    }
    if named("seiso.toml") {
        return Ok(!candidate_exists(&target_dir.join(".seiso.toml"))?);
    }
    if named("pyproject.toml") {
        return Ok(!candidate_exists(&target_dir.join(".seiso.toml"))?
            && !candidate_exists(&target_dir.join("seiso.toml"))?);
    }
    Ok(false)
}

/// Probe only higher-precedence candidates; the target itself is read next.
fn candidate_exists(path: &Path) -> Result<bool, ConfigError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(invalid(
            path,
            "expected a configuration file, found a directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Consume an additive string list without discarding its per-entry frames.
fn take_string_extension_list(
    value: &mut FramedValue,
    field: &str,
    path: &Path,
) -> Result<Vec<(String, FrameId)>, ConfigError> {
    let Some(value) = value.remove(field) else {
        return Ok(Vec::new());
    };
    let FramedKind::Array(entries) = value.kind else {
        return Err(invalid(
            path,
            format!("{field} must be an array of strings"),
        ));
    };
    entries
        .into_iter()
        .map(|entry| {
            entry
                .string()
                .map(|string| (string.to_owned(), entry.frame))
                .ok_or_else(|| invalid(path, format!("{field} must contain only strings")))
        })
        .collect()
}

/// Consume additive mapping entries, decoding only the surviving effective list.
fn take_mapping_extension_list<T: serde::de::DeserializeOwned>(
    value: &mut FramedValue,
    field: &str,
    path: &Path,
) -> Result<Vec<(T, FrameId)>, ConfigError> {
    let Some(value) = value.remove(field) else {
        return Ok(Vec::new());
    };
    let FramedKind::Array(entries) = value.kind else {
        return Err(invalid(
            path,
            format!("{field} must be an array of mappings"),
        ));
    };
    entries
        .into_iter()
        .map(|entry| {
            let frame = entry.frame;
            let mapping = entry
                .into_toml()
                .try_into()
                .map_err(|error| invalid(path, format!("invalid {field} entry: {error}")))?;
            Ok((mapping, frame))
        })
        .collect()
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

/// Render a frame path without embedding machine-specific workspace prefixes.
fn relative_label(root: &Path, path: &Path) -> String {
    for ancestor in root.ancestors() {
        if let Ok(suffix) = path.strip_prefix(ancestor) {
            let mut relative = PathBuf::new();
            for _ in root
                .strip_prefix(ancestor)
                .into_iter()
                .flat_map(Path::components)
            {
                relative.push("..");
            }
            relative.push(suffix);
            let text = relative.to_string_lossy().replace('\\', "/");
            return if text.is_empty() {
                ".".to_owned()
            } else {
                text
            };
        }
    }
    path.to_string_lossy().replace('\\', "/")
}

fn absolute(path: &Path) -> Result<PathBuf, ConfigError> {
    std::path::absolute(path)
        .map(normalize)
        .map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })
}
