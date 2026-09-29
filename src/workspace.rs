//! Discover and load the inputs needed by an inspection or a check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::ParseCache;
use crate::config::{CliOverrides, Config, Settings, SiteMapping, Workspace};
use crate::index::{IndexedFile, WorkspaceIndex};
use crate::md::Document;
use crate::paths::normalize;
use crate::rules::{Kind, KindOutcome, SuppressionRecord, resolve_kind};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputError {
    pub filename: String,
    pub message: String,
}

#[derive(Default)]
pub struct LoadOptions {
    pub paths: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub overrides: CliOverrides,
    pub stdin: Option<(PathBuf, String)>,
    pub no_cache: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum LoadScope {
    /// Parse only the requested documents.
    Selected,
    /// Include workspace dependencies when an included file can run index rules.
    Check,
    /// Inspect every included document.
    Workspace,
}

#[derive(Clone, Debug, Serialize)]
pub struct FilePolicy {
    pub filename: String,
    pub configuration: String,
    pub kind: Option<KindOutcome>,
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteMapping>,
    pub enabled_rules: Vec<String>,
    pub excluded: Option<&'static str>,
    pub suppressions: Vec<SuppressionRecord>,
}

impl FilePolicy {
    pub fn resolve(
        filename: String,
        path: &Path,
        document: &Document,
        config: &Config,
        overrides: &CliOverrides,
    ) -> Result<Self, crate::config::ConfigError> {
        let kind = resolve_kind(document, config.kind_for(path));
        let enabled_rules = config
            .enabled_rules(path, kind.value(), overrides)?
            .into_iter()
            .map(str::to_owned)
            .collect();
        Ok(Self {
            filename,
            configuration: config
                .source
                .as_ref()
                .map(|source| relative(&config.directory, source))
                .unwrap_or_else(|| "<defaults>".into()),
            kind: Some(kind),
            domain: config.domain_for(path).map(str::to_owned),
            site: config.site_for(path).cloned(),
            enabled_rules,
            excluded: None,
            suppressions: Vec::new(),
        })
    }

    pub fn kind_value(&self) -> Option<Kind> {
        self.kind.as_ref().and_then(KindOutcome::value)
    }
}

pub struct Snapshot {
    pub index: WorkspaceIndex,
    pub selected: BTreeSet<String>,
    pub requested: BTreeSet<PathBuf>,
    pub configurations: BTreeMap<String, Settings>,
    pub excluded_policies: BTreeMap<String, FilePolicy>,
    pub errors: Vec<InputError>,
    /// Requested paths, or the whole workspace, that selected no document to check.
    pub skipped: Vec<InputError>,
}

impl Snapshot {
    pub fn policies(&self) -> BTreeMap<&str, &FilePolicy> {
        self.index
            .files()
            .iter()
            .map(|file| (file.policy.filename.as_str(), &file.policy))
            .chain(
                self.excluded_policies
                    .iter()
                    .map(|(name, policy)| (name.as_str(), policy)),
            )
            .collect()
    }

    pub fn enabled_count(&self) -> usize {
        self.index
            .files()
            .iter()
            .filter(|file| self.selected.contains(&file.policy.filename))
            .map(|file| file.policy.enabled_rules.len())
            .sum()
    }

    pub fn sort_errors(&mut self) {
        self.errors
            .sort_by(|a, b| (&a.filename, &a.message).cmp(&(&b.filename, &b.message)));
        self.errors.dedup();
    }
}

struct PendingDocument {
    path: PathBuf,
    filename: String,
    config: Config,
    configuration: String,
    source_override: Option<String>,
}

fn load_documents(
    inputs: &[PendingDocument],
    cache: &ParseCache,
) -> Vec<Result<Arc<Document>, String>> {
    let load = |input: &PendingDocument| {
        let source = input
            .source_override
            .as_ref()
            .map_or_else(
                || fs::read_to_string(&input.path),
                |source| Ok(source.clone()),
            )
            .map_err(|error| format!("Cannot read UTF-8 Markdown: {error}"))?;
        cache
            .parse_shared(&source)
            .map_err(|error| format!("Cannot parse Markdown: {error}"))
    };
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(4);
    if workers == 1 || inputs.len() < 32 {
        return inputs.iter().map(load).collect();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .chunks(inputs.len().div_ceil(workers))
            .map(|batch| {
                let load = &load;
                (
                    batch.len(),
                    scope.spawn(move || batch.iter().map(load).collect::<Vec<_>>()),
                )
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|(count, handle)| {
                handle.join().unwrap_or_else(|_| {
                    (0..count)
                        .map(|_| Err("Markdown worker did not finish; rerun the check.".to_owned()))
                        .collect()
                })
            })
            .collect()
    })
}

/// Resolve file policies before deciding which sources a check needs to read.
pub fn load(cwd: &Path, options: &LoadOptions, scope: LoadScope) -> Result<Snapshot, String> {
    options
        .overrides
        .validate()
        .map_err(|error| error.to_string())?;
    let workspace =
        Workspace::discover(cwd, options.config.as_deref()).map_err(|error| error.to_string())?;
    let root = normalize(&workspace.root);
    let mut errors = Vec::new();
    let mut request = resolve_request(cwd, &root, options, &mut errors)?;
    let discovery = discover(&root, request.overlay.take());
    let mut plan = LoadPlan::resolve(
        &workspace,
        &root,
        options,
        scope,
        &request,
        discovery.inputs,
        errors,
    )?;
    plan.select_dependencies(&request, discovery.errors);
    let cache = ParseCache::new(root.join(".seiso_cache"), !options.no_cache);
    let loaded = load_documents(&plan.pending, &cache);
    plan.assemble(root, options, request, loaded)
}

struct Request {
    paths: BTreeSet<PathBuf>,
    all_selected: bool,
    overlay: Option<(PathBuf, String)>,
}

impl Request {
    fn includes(&self, path: &Path) -> bool {
        self.all_selected || self.paths.iter().any(|selected| path.starts_with(selected))
    }
}

fn resolve_request(
    cwd: &Path,
    root: &Path,
    options: &LoadOptions,
    errors: &mut Vec<InputError>,
) -> Result<Request, String> {
    let mut requested = BTreeSet::new();
    let overlay = if let Some((path, source)) = &options.stdin {
        let path = workspace_path(root, &absolute(cwd, path))?;
        if !is_markdown(&path) {
            return Err("The stdin filename must have a .md or .markdown extension.".into());
        }
        if ignored_by_git(root, &path, false)? {
            return Err(format!(
                "{} is excluded by .gitignore; choose an included Markdown path.",
                relative(root, &path)
            ));
        }
        requested.insert(path.clone());
        Some((path, source.clone()))
    } else {
        for path in &options.paths {
            let path = workspace_path(root, &absolute(cwd, path))?;
            match path.try_exists() {
                Ok(true) => {
                    requested.insert(path);
                }
                Ok(false) => errors.push(InputError {
                    filename: relative(root, &path),
                    message: "Path does not exist; provide an existing file or directory.".into(),
                }),
                Err(error) => errors.push(InputError {
                    filename: relative(root, &path),
                    message: format!("Cannot access this path: {error}"),
                }),
            }
        }
        None
    };
    let all_selected = options.paths.is_empty() && options.stdin.is_none();
    Ok(Request {
        paths: requested,
        all_selected,
        overlay,
    })
}

struct Discovery {
    inputs: BTreeMap<PathBuf, Option<String>>,
    errors: Vec<(Option<PathBuf>, InputError)>,
}

fn discover(root: &Path, overlay: Option<(PathBuf, String)>) -> Discovery {
    let mut inputs = BTreeMap::new();
    let mut discovery_errors = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git" && entry.file_name() != ".seiso_cache")
        .build();
    for entry in walker {
        match entry {
            Ok(entry) => {
                let path = normalize(entry.path());
                if let Some(error) = entry.error() {
                    discovery_errors.push((
                        Some(path.clone()),
                        InputError {
                            filename: relative(root, &path),
                            message: format!("Cannot fully apply ignore rules: {error}"),
                        },
                    ));
                }
                if entry.file_type().is_some_and(|kind| kind.is_file()) && is_markdown(&path) {
                    inputs.insert(path, None);
                }
            }
            Err(error) => discovery_errors.push((
                discovery_error_path(&error).map(Path::to_owned),
                InputError {
                    filename: ".".into(),
                    message: format!("Cannot finish workspace discovery: {error}"),
                },
            )),
        }
    }
    if let Some((path, source)) = overlay {
        inputs.insert(path, Some(source));
    }
    Discovery {
        inputs,
        errors: discovery_errors,
    }
}

struct LoadPlan {
    pending: Vec<PendingDocument>,
    configurations: BTreeMap<String, Settings>,
    policies: BTreeMap<String, FilePolicy>,
    errors: Vec<InputError>,
    deferred_errors: Vec<InputError>,
    excluded_inputs: Vec<(PathBuf, &'static str, String)>,
    needs_index: bool,
}

impl LoadPlan {
    fn resolve(
        workspace: &Workspace,
        root: &Path,
        options: &LoadOptions,
        scope: LoadScope,
        request: &Request,
        inputs: BTreeMap<PathBuf, Option<String>>,
        mut errors: Vec<InputError>,
    ) -> Result<Self, String> {
        let mut configurations = BTreeMap::new();
        record_configuration(
            &mut configurations,
            root,
            &workspace.config,
            &options.overrides,
        );
        let mut policies = BTreeMap::new();
        let mut config_cache = BTreeMap::new();
        let mut pending = Vec::new();
        let mut deferred_errors = Vec::new();
        let mut excluded_inputs = Vec::new();
        let mut needs_index = scope == LoadScope::Workspace;
        for (path, source_override) in inputs {
            let selected = request.includes(&path);
            if scope == LoadScope::Selected && !selected {
                continue;
            }
            let filename = relative(root, &path);
            let directory = path.parent().unwrap_or(root).to_path_buf();
            let config = match config_cache.entry(directory).or_insert_with(|| {
                workspace
                    .config_for(&path)
                    .map_err(|error| error.to_string())
            }) {
                Ok(config) => config.clone(),
                Err(error) => {
                    let error = InputError {
                        filename,
                        message: error.clone(),
                    };
                    if selected || scope == LoadScope::Workspace {
                        errors.push(error);
                    } else {
                        deferred_errors.push(error);
                    }
                    continue;
                }
            };
            let configuration =
                record_configuration(&mut configurations, root, &config, &options.overrides);
            let excluded = if !config.includes(&path) {
                Some("include")
            } else if config.excludes(&path) {
                Some("exclude")
            } else {
                None
            };
            if let Some(field) = excluded {
                if selected {
                    excluded_inputs.push((path.clone(), field, configuration.clone()));
                }
                if source_override.is_some() {
                    errors.push(InputError {
                    filename: filename.clone(),
                    message: "The stdin filename is excluded by configuration; choose an included Markdown path.".into(),
                });
                }
                if scope == LoadScope::Workspace {
                    policies.insert(
                        filename.clone(),
                        FilePolicy {
                            filename,
                            configuration,
                            kind: None,
                            domain: config.domain_for(&path).map(str::to_owned),
                            site: config.site_for(&path).cloned(),
                            enabled_rules: Vec::new(),
                            excluded,
                            suppressions: Vec::new(),
                        },
                    );
                }
                continue;
            }
            needs_index |= requires_index(scope, &config, &path, &options.overrides)?;
            pending.push(PendingDocument {
                path,
                filename,
                config,
                configuration,
                source_override,
            });
        }
        Ok(Self {
            pending,
            configurations,
            policies,
            errors,
            deferred_errors,
            excluded_inputs,
            needs_index,
        })
    }

    fn select_dependencies(
        &mut self,
        request: &Request,
        discovery_errors: Vec<(Option<PathBuf>, InputError)>,
    ) {
        if self.needs_index {
            self.errors.append(&mut self.deferred_errors);
        }
        self.errors.extend(
            discovery_errors
                .into_iter()
                .filter(|(path, _)| {
                    self.needs_index
                        || request.all_selected
                        || path.as_ref().is_none_or(|path| {
                            request.paths.iter().any(|selected| {
                                path.starts_with(selected) || selected.starts_with(path)
                            })
                        })
                })
                .map(|(_, error)| error),
        );
        self.pending
            .retain(|input| self.needs_index || request.includes(&input.path));
    }

    fn assemble(
        mut self,
        root: PathBuf,
        options: &LoadOptions,
        request: Request,
        loaded: Vec<Result<Arc<Document>, String>>,
    ) -> Result<Snapshot, String> {
        let mut files = Vec::new();
        let mut selected = BTreeSet::new();
        for (input, loaded) in self.pending.into_iter().zip(loaded) {
            let PendingDocument {
                path,
                filename,
                config,
                configuration,
                ..
            } = input;
            let document = match loaded {
                Ok(document) => document,
                Err(message) => {
                    self.errors.push(InputError { filename, message });
                    continue;
                }
            };
            let mut policy = FilePolicy::resolve(
                filename.clone(),
                &path,
                &document,
                &config,
                &options.overrides,
            )
            .map_err(|error| error.to_string())?;
            policy.configuration = configuration;
            if request.includes(&path) {
                selected.insert(filename.clone());
            }
            files.push(IndexedFile {
                policy,
                path,
                document,
                config,
            });
        }
        let checked: Vec<_> = files
            .iter()
            .filter(|file| selected.contains(&file.policy.filename))
            .map(|file| file.path.as_path())
            .collect();
        let skipped = skipped_inputs(
            &root,
            &request.paths,
            request.all_selected,
            &checked,
            &self.excluded_inputs,
            &self.errors,
        );
        let index = WorkspaceIndex::new(root, files, self.errors.is_empty());
        let mut snapshot = Snapshot {
            index,
            selected,
            requested: request.paths,
            configurations: self.configurations,
            excluded_policies: self.policies,
            errors: self.errors,
            skipped,
        };
        snapshot.sort_errors();
        Ok(snapshot)
    }
}

fn requires_index(
    scope: LoadScope,
    config: &Config,
    path: &Path,
    overrides: &CliOverrides,
) -> Result<bool, String> {
    Ok(scope == LoadScope::Check
        && config
            .selected_rules(path, overrides)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|code| crate::rules::rule(code).is_some_and(|rule| rule.requires_index)))
}

/// Explain each requested path, or an empty workspace, that selected no document.
fn skipped_inputs(
    root: &Path,
    requested: &BTreeSet<PathBuf>,
    all_selected: bool,
    checked: &[&Path],
    excluded: &[(PathBuf, &'static str, String)],
    errors: &[InputError],
) -> Vec<InputError> {
    let targets: Vec<&Path> = if all_selected {
        vec![root]
    } else {
        requested.iter().map(PathBuf::as_path).collect()
    };
    let mut skipped = Vec::new();
    for target in targets {
        if checked.iter().any(|path| path.starts_with(target))
            || errors
                .iter()
                .any(|error| root.join(&error.filename).starts_with(target))
        {
            continue;
        }
        let excluded_here: Vec<_> = excluded
            .iter()
            .filter(|(path, ..)| path.starts_with(target))
            .collect();
        let reason = if target.is_file() {
            if !is_markdown(target) {
                "it is not a .md or .markdown file".to_owned()
            } else if let Some((_, field, configuration)) = excluded_here.first() {
                let configuration = configuration_label(configuration);
                if *field == "exclude" {
                    format!("`exclude` in {configuration} matches it")
                } else {
                    format!("`include` in {configuration} does not match it")
                }
            } else if ignored_by_git(root, target, false).unwrap_or(false) {
                ".gitignore ignores it".to_owned()
            } else {
                "workspace discovery skipped it; symbolic links are not followed".to_owned()
            }
        } else if !excluded_here.is_empty() {
            let count = excluded_here.len();
            let files = if count == 1 { "file" } else { "files" };
            format!("configuration excludes its {count} Markdown {files}; inspect `seiso policy`")
        } else if target != root && ignored_by_git(root, target, true).unwrap_or(false) {
            ".gitignore ignores it".to_owned()
        } else if contains_markdown_ignored_by_git(target) {
            ".gitignore ignores its Markdown files".to_owned()
        } else {
            "it contains no Markdown files".to_owned()
        };
        skipped.push(if target == root {
            InputError {
                filename: ".".into(),
                message: format!(
                    "No documents were checked in workspace {} because {reason}.",
                    root.display()
                ),
            }
        } else {
            InputError {
                filename: relative(root, target),
                message: format!("Not checked because {reason}."),
            }
        });
    }
    skipped
}

/// Discovery drops Git-ignored files, so find them again only to explain a
/// directory that selected nothing. Other discovery settings match `load`.
fn contains_markdown_ignored_by_git(directory: &Path) -> bool {
    ignore::WalkBuilder::new(directory)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git" && entry.file_name() != ".seiso_cache")
        .build()
        .flatten()
        .any(|entry| {
            entry.file_type().is_some_and(|kind| kind.is_file()) && is_markdown(entry.path())
        })
}

fn configuration_label(configuration: &str) -> String {
    if configuration == "<defaults>" {
        "the default configuration".into()
    } else {
        configuration.to_owned()
    }
}

fn discovery_error_path(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            discovery_error_path(err)
        }
        ignore::Error::Loop { child, .. } => Some(child),
        ignore::Error::Partial(errors) if errors.len() == 1 => discovery_error_path(&errors[0]),
        _ => None,
    }
}

fn record_configuration(
    configurations: &mut BTreeMap<String, Settings>,
    root: &Path,
    config: &Config,
    overrides: &CliOverrides,
) -> String {
    let name = config
        .source
        .as_ref()
        .map(|path| relative(root, path))
        .unwrap_or_else(|| "<defaults>".into());
    configurations.entry(name.clone()).or_insert_with(|| {
        let mut settings = config.settings.clone();
        settings.preview |= overrides.preview;
        if let Some(select) = &overrides.select {
            settings.lint.select.clone_from(select);
        }
        settings.lint.select.extend(overrides.extend_select.clone());
        settings
    });
    name
}

pub fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    normalize(&if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    })
}

pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

fn ignored_by_git(root: &Path, path: &Path, path_is_directory: bool) -> Result<bool, String> {
    let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
    let components = relative.components().collect::<Vec<_>>();
    let mut directory = root.to_path_buf();
    let mut matchers = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if component.as_os_str() == ".git" {
            return Ok(true);
        }
        let ignore_file = directory.join(".gitignore");
        if ignore_file.is_file() {
            let mut builder = ignore::gitignore::GitignoreBuilder::new(&directory);
            if let Some(error) = builder.add(&ignore_file) {
                return Err(format!("Cannot read {}: {error}", ignore_file.display()));
            }
            matchers.push(
                builder
                    .build()
                    .map_err(|error| format!("Cannot read Git ignore patterns: {error}"))?,
            );
        }
        directory.push(component.as_os_str());
        let is_directory = index + 1 < components.len() || path_is_directory;
        for matcher in matchers.iter().rev() {
            let matched = matcher.matched(&directory, is_directory);
            if matched.is_ignore() {
                return Ok(true);
            }
            if matched.is_whitelist() {
                break;
            }
        }
    }
    Ok(false)
}

/// Resolve existing aliases, including the parent of a new stdin document.
fn workspace_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve workspace {}: {error}", root.display()))?;
    for ancestor in path.ancestors() {
        let resolved = match ancestor.canonicalize() {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot resolve {}: {error}", path.display())),
        };
        let suffix = path
            .strip_prefix(ancestor)
            .map_err(|error| error.to_string())?;
        if !suffix.as_os_str().is_empty() && !ancestor.is_dir() {
            return Err(format!(
                "{} is not a directory; choose a valid workspace path.",
                ancestor.display()
            ));
        }
        let resolved = resolved.join(suffix);
        return resolved
            .strip_prefix(&canonical_root)
            .map(|relative| root.join(relative))
            .map_err(|_| {
                format!(
                    "{} is outside workspace {}; run from its workspace directory.",
                    path.display(),
                    root.display()
                )
            });
    }
    Err(format!(
        "Cannot resolve {} to an existing workspace directory.",
        path.display()
    ))
}

pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
