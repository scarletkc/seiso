//! Discover and load the inputs needed by an inspection or a check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cache::ParseCache;
use crate::config::{CliOverrides, Config, EnvironmentReport, Settings, SiteMapping, Workspace};
use crate::index::{IndexedFile, WorkspaceIndex};
use crate::md::Document;
use crate::paths::normalize;
use crate::rules::{KindResolution, SuppressionRecord, resolve_kind};
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
    /// Build serializable frame provenance only for consumers that display it.
    pub explain_environment: bool,
    /// Load project-wide dependencies even when the selected policy needs none.
    pub project_dependencies: bool,
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

#[derive(Clone, Serialize)]
pub struct FilePolicy {
    pub filename: String,
    pub configuration: String,
    pub kind: Option<KindResolution>,
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteMapping>,
    pub enabled_rules: Vec<String>,
    pub excluded: Option<&'static str>,
    pub suppressions: Vec<SuppressionRecord>,
}

pub struct Snapshot {
    pub index: WorkspaceIndex,
    /// Root that defines user-facing selected paths, distinct from index.root.
    pub selection_root: PathBuf,
    /// The invoking directory's policy, retained even when no file is selected.
    pub invocation_configuration: String,
    pub selected: BTreeSet<String>,
    pub requested: BTreeSet<PathBuf>,
    pub configurations: BTreeMap<String, Settings>,
    /// Pattern provenance keyed identically to `configurations`.
    pub environments: BTreeMap<String, EnvironmentReport>,
    pub policies: BTreeMap<String, FilePolicy>,
    pub errors: Vec<InputError>,
    /// Requested paths, or the whole workspace, that selected no document to check.
    pub skipped: Vec<InputError>,
}

impl Snapshot {
    /// Render an internal project-relative file ID from the selection root.
    pub fn display_name(&self, filename: &str) -> String {
        display_name(&self.selection_root, &self.index.root, filename)
    }
    pub fn enabled_count(&self) -> usize {
        self.index
            .files()
            .iter()
            .filter(|file| self.selected.contains(&file.filename))
            .map(|file| file.enabled_rules.len())
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

/// Discover Markdown under one root without following symlinks or cache files.
fn walk_markdown(
    root: &Path,
    inputs: &mut BTreeMap<PathBuf, Option<String>>,
    errors: &mut Vec<(Option<PathBuf>, InputError)>,
    project_root: &Path,
) {
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
                    errors.push((
                        Some(path.clone()),
                        InputError {
                            filename: relative(project_root, &path),
                            message: format!("Cannot fully apply ignore rules: {error}"),
                        },
                    ));
                }
                if entry.file_type().is_some_and(|kind| kind.is_file()) && is_markdown(&path) {
                    inputs.entry(path).or_insert(None);
                }
            }
            Err(error) => errors.push((
                discovery_error_path(&error).map(Path::to_owned),
                InputError {
                    filename: ".".into(),
                    message: format!("Cannot finish workspace discovery: {error}"),
                },
            )),
        }
    }
}

/// Expand only from selected files' governing frames, caching each directory.
///
/// Repeated calls are needed only when an explicit outside path is admitted by
/// an earlier selected frame; ordinary child invocations make one pass.
fn expand_selected_scope(
    workspace: &Workspace,
    selection_root: &Path,
    requested: &BTreeSet<PathBuf>,
    all_selected: bool,
    inputs: &BTreeMap<PathBuf, Option<String>>,
    project_root: &mut PathBuf,
    config_cache: &mut BTreeMap<PathBuf, Result<Config, String>>,
) {
    for path in inputs.keys().filter(|path| {
        (all_selected && path.starts_with(selection_root))
            || requested.iter().any(|selected| path.starts_with(selected))
    }) {
        let directory = path.parent().unwrap_or(selection_root).to_path_buf();
        let config = config_cache.entry(directory).or_insert_with(|| {
            workspace
                .config_for_within(path, project_root)
                .map_err(|error| error.to_string())
        });
        if let Ok(config) = config {
            let admitted = config.project_root_for_source();
            if project_root.starts_with(&admitted) {
                *project_root = admitted;
            }
        }
    }
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
    let mut project_root = normalize(&workspace.project_root);
    let invocation_project_root = project_root.clone();
    let mut errors = Vec::new();
    let mut path_errors = Vec::new();
    let mut requested = BTreeSet::new();
    let mut deferred_requests = Vec::new();
    let mut inputs = BTreeMap::new();
    let overlay = if let Some((path, source)) = &options.stdin {
        let path = workspace_path(&project_root, &absolute(cwd, path))?;
        if !is_markdown(&path) {
            return Err("The stdin filename must have a .md or .markdown extension.".into());
        }
        if ignored_by_git(&project_root, &path, false)? {
            return Err(format!(
                "{} is excluded by .gitignore; choose an included Markdown path.",
                relative(&root, &path)
            ));
        }
        requested.insert(path.clone());
        Some((path, source.clone()))
    } else {
        for path in &options.paths {
            let absolute = absolute(cwd, path);
            let path = match workspace_path(&project_root, &absolute) {
                Ok(path) => path,
                Err(_) if !absolute.starts_with(&project_root) => {
                    deferred_requests.push(absolute);
                    continue;
                }
                Err(error) => return Err(error),
            };
            match path.try_exists() {
                Ok(true) => {
                    requested.insert(path);
                }
                Ok(false) => path_errors.push((
                    path,
                    "Path does not exist; provide an existing file or directory.".to_owned(),
                )),
                Err(error) => path_errors.push((path, format!("Cannot access this path: {error}"))),
            }
        }
        None
    };
    let all_selected = options.paths.is_empty() && options.stdin.is_none();
    let mut discovery_errors = Vec::new();
    walk_markdown(&root, &mut inputs, &mut discovery_errors, &project_root);
    for target in requested.iter().filter(|path| !path.starts_with(&root)) {
        if target.is_file() && is_markdown(target) {
            if !ignored_by_git(&project_root, target, false)? {
                inputs.entry(target.clone()).or_insert(None);
            }
        } else if target.is_dir() {
            walk_markdown(target, &mut inputs, &mut discovery_errors, &project_root);
        }
    }
    if let Some((path, source)) = overlay {
        inputs.insert(path, Some(source));
    }
    let mut config_cache = BTreeMap::new();
    // Selected frames, never incidental dependency files, can widen scope.
    expand_selected_scope(
        &workspace,
        &root,
        &requested,
        all_selected,
        &inputs,
        &mut project_root,
        &mut config_cache,
    );
    // An explicit parent/sibling request may become legal because another
    // selected file's nested frame has just admitted its governing ancestor.
    // Do not let the outside request itself promote an unrelated workspace.
    let mut deferred = deferred_requests;
    while !deferred.is_empty() {
        let mut unresolved = Vec::new();
        let mut admitted_any = false;
        for absolute in deferred {
            let path = match workspace_path(&project_root, &absolute) {
                Ok(path) => path,
                Err(error) => {
                    unresolved.push((absolute, error));
                    continue;
                }
            };
            admitted_any = true;
            match path.try_exists() {
                Ok(true) => {
                    requested.insert(path.clone());
                    if path.is_file() && is_markdown(&path) {
                        if !ignored_by_git(&project_root, &path, false)? {
                            inputs.entry(path).or_insert(None);
                        }
                    } else if path.is_dir() {
                        walk_markdown(&path, &mut inputs, &mut discovery_errors, &project_root);
                    }
                }
                Ok(false) => path_errors.push((
                    path,
                    "Path does not exist; provide an existing file or directory.".to_owned(),
                )),
                Err(error) => path_errors.push((path, format!("Cannot access this path: {error}"))),
            }
        }
        if !admitted_any {
            return Err(unresolved.remove(0).1);
        }
        expand_selected_scope(
            &workspace,
            &root,
            &requested,
            all_selected,
            &inputs,
            &mut project_root,
            &mut config_cache,
        );
        deferred = unresolved.into_iter().map(|(path, _)| path).collect();
    }
    errors.extend(path_errors.into_iter().map(|(path, message)| InputError {
        filename: relative(&project_root, &path),
        message,
    }));
    for (path, error) in &mut discovery_errors {
        if let Some(path) = path {
            error.filename = relative(&project_root, path);
        }
    }
    let selected_path = |path: &Path| {
        (all_selected && path.starts_with(&root))
            || requested.iter().any(|selected| path.starts_with(selected))
    };
    // The old single-root path needs no preflight: its first walk already
    // discovered every possible dependency. Skip duplicate policy resolution.
    let selected_requires_index = project_root != root
        && scope == LoadScope::Check
        && inputs
            .keys()
            .filter(|path| selected_path(path))
            .try_fold(false, |required, path| {
                if required {
                    return Ok::<_, String>(true);
                }
                // Defer malformed nested configs to the normal per-file error
                // path so one bad directory does not hide unrelated failures.
                let Some(Ok(config)) = config_cache.get(path.parent().unwrap_or(&root)) else {
                    return Ok(false);
                };
                if !config.includes(path) || config.excludes(path) {
                    return Ok(false);
                }
                let rules = config
                    .selected_rules(path, &options.overrides)
                    .map_err(|error| error.to_string())?;
                Ok(rules
                    .iter()
                    .any(|code| crate::rules::rule(code).is_some_and(|rule| rule.requires_index)))
            })?;
    let project_dependencies = options.project_dependencies || selected_requires_index;
    if project_dependencies && project_root != root {
        walk_markdown(
            &project_root,
            &mut inputs,
            &mut discovery_errors,
            &project_root,
        );
    }
    let mut configurations = BTreeMap::new();
    let mut environments = BTreeMap::new();
    let invocation_configuration = record_configuration(
        &mut configurations,
        &mut environments,
        &root,
        &workspace.config,
        &options.overrides,
        options.explain_environment,
    );
    let mut policies = BTreeMap::new();
    let mut pending = Vec::new();
    let mut deferred_errors = Vec::new();
    let mut excluded_inputs = Vec::new();
    let mut needs_index = scope == LoadScope::Workspace || project_dependencies;
    for (path, source_override) in inputs {
        let selected = selected_path(&path);
        if scope == LoadScope::Selected && !selected {
            continue;
        }
        let filename = relative(&project_root, &path);
        let directory = path.parent().unwrap_or(&root).to_path_buf();
        let config = match config_cache.entry(directory).or_insert_with(|| {
            workspace
                .config_for_within(&path, &project_root)
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
        let configuration = record_configuration(
            &mut configurations,
            &mut environments,
            &root,
            &config,
            &options.overrides,
            options.explain_environment,
        );
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
                errors.push(InputError {filename:filename.clone(),message:"The stdin filename is excluded by configuration; choose an included Markdown path.".into()});
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
        if scope == LoadScope::Check
            && config
                .selected_rules(&path, &options.overrides)
                .map_err(|error| error.to_string())?
                .iter()
                .any(|code| crate::rules::rule(code).is_some_and(|rule| rule.requires_index))
        {
            needs_index = true;
        }
        pending.push(PendingDocument {
            path,
            filename,
            config,
            configuration,
            source_override,
        });
    }
    if needs_index {
        errors.extend(deferred_errors);
    }
    errors.extend(
        discovery_errors
            .into_iter()
            .filter(|(path, _)| {
                needs_index
                    || all_selected
                    || path.as_ref().is_none_or(|path| {
                        requested.iter().any(|selected| {
                            path.starts_with(selected) || selected.starts_with(path)
                        })
                    })
            })
            .map(|(_, error)| error),
    );
    pending.retain(|input| needs_index || selected_path(&input.path));
    let cache = ParseCache::new(root.join(".seiso_cache"), !options.no_cache);
    let loaded = load_documents(&pending, &cache);
    let mut files = Vec::new();
    let mut selected = BTreeSet::new();
    for (input, loaded) in pending.into_iter().zip(loaded) {
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
                errors.push(InputError { filename, message });
                continue;
            }
        };
        let kind = resolve_kind(&document, config.kind_for(&path));
        let enabled_rules = config
            .enabled_rules(&path, kind.value.as_deref(), &options.overrides)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if selected_path(&path) {
            selected.insert(filename.clone());
        }
        policies.insert(
            filename.clone(),
            FilePolicy {
                filename: filename.clone(),
                configuration,
                kind: Some(kind.clone()),
                domain: config.domain_for(&path).map(str::to_owned),
                site: config.site_for(&path).cloned(),
                enabled_rules: enabled_rules.clone(),
                excluded: None,
                suppressions: Vec::new(),
            },
        );
        files.push(IndexedFile {
            filename,
            path: path.clone(),
            document,
            kind: kind.value,
            domain: config.domain_for(&path).unwrap_or("").to_owned(),
            enabled_rules,
            config,
        });
    }
    let checked: Vec<_> = files
        .iter()
        .filter(|file| selected.contains(&file.filename))
        .map(|file| file.path.as_path())
        .collect();
    let skipped = skipped_inputs(
        &root,
        &project_root,
        &requested,
        all_selected,
        &checked,
        &excluded_inputs,
        &errors,
    );
    let complete =
        needs_index && (project_root == root || project_dependencies) && errors.is_empty();
    let index = WorkspaceIndex::new_with_scope(
        project_root,
        root.clone(),
        invocation_project_root,
        files,
        complete,
    );
    let mut snapshot = Snapshot {
        index,
        selection_root: root,
        invocation_configuration,
        selected,
        requested,
        configurations,
        environments,
        policies,
        errors,
        skipped,
    };
    snapshot.sort_errors();
    Ok(snapshot)
}

/// Explain each requested path, or an empty workspace, that selected no document.
fn skipped_inputs(
    root: &Path,
    project_root: &Path,
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
                .any(|error| project_root.join(&error.filename).starts_with(target))
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
            } else if ignored_by_git(project_root, target, false).unwrap_or(false) {
                ".gitignore ignores it".to_owned()
            } else {
                "workspace discovery skipped it; symbolic links are not followed".to_owned()
            }
        } else if !excluded_here.is_empty() {
            let count = excluded_here.len();
            let files = if count == 1 { "file" } else { "files" };
            format!("configuration excludes its {count} Markdown {files}; inspect `seiso policy`")
        } else if target != root && ignored_by_git(project_root, target, true).unwrap_or(false) {
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
    environments: &mut BTreeMap<String, EnvironmentReport>,
    root: &Path,
    config: &Config,
    overrides: &CliOverrides,
    explain_environment: bool,
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
    if explain_environment {
        environments
            .entry(name.clone())
            .or_insert_with(|| config.environment_report(root));
    }
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

/// Convert a project-relative internal ID into an invocation-relative label.
fn display_name(selection_root: &Path, project_root: &Path, filename: &str) -> String {
    let path = project_root.join(filename);
    let from: Vec<_> = selection_root.components().collect();
    let to: Vec<_> = path.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..from.len() {
        relative.push("..");
    }
    for component in &to[common..] {
        relative.push(component.as_os_str());
    }
    let label = relative.to_string_lossy().replace('\\', "/");
    if label.is_empty() { ".".into() } else { label }
}
