use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, Subcommand, ValueEnum};
use seiso::analysis::{self, Analysis};
use seiso::config::{CliOverrides, Settings, Workspace, repository_root};
use seiso::diagnostics::{
    Diagnostic, render_concise, render_github, render_json, render_sarif, render_text,
};
use seiso::index::IndexedFile;
use seiso::md::Document;
use seiso::rules::KindResolution;
use seiso::workspace::{
    self, FilePolicy, InputError, LoadOptions, LoadScope, PolicyRecord, Snapshot, is_markdown,
};
use serde::Serialize;

#[derive(Args, Default)]
pub struct SelectionArgs {
    /// Enable selected preview rules.
    #[arg(long)]
    preview: bool,
    /// Replace configured selectors; use comma-separated codes or families.
    #[arg(long, value_delimiter = ',')]
    select: Option<Vec<String>>,
    /// Add comma-separated codes or families to the configured selection.
    #[arg(long, value_delimiter = ',')]
    extend_select: Vec<String>,
}

impl SelectionArgs {
    fn overrides(&self) -> CliOverrides {
        CliOverrides {
            preview: self.preview,
            select: self.select.clone(),
            extend_select: self.extend_select.clone(),
        }
    }
}

#[derive(Args)]
pub struct CheckArgs {
    /// Files or directories to check; defaults to the workspace.
    paths: Vec<PathBuf>,
    /// Use this configuration for every selected file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(flatten)]
    selection: SelectionArgs,
    /// Diagnostic output format.
    #[arg(long, value_enum, default_value_t = CheckFormat::Text)]
    output_format: CheckFormat,
    /// Read stdin in place of this workspace file without writing to disk; excludes --fix.
    #[arg(long, value_name = "PATH", conflicts_with = "paths")]
    stdin_filename: Option<PathBuf>,
    /// Return success for violations; incomplete checks still return 2.
    #[arg(long)]
    exit_zero: bool,
    /// Read required source files without reading or writing the parse cache.
    #[arg(long)]
    no_cache: bool,
    /// Apply verified safe fixes to reported files.
    #[arg(long, conflicts_with = "stdin_filename")]
    fix: bool,
    /// Count reported diagnostics and include suppression reasons and states.
    #[arg(long)]
    statistics: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum CheckFormat {
    Text,
    Concise,
    Json,
    Sarif,
    Github,
}

#[derive(Args)]
pub struct IndexArgs {
    /// Print the current workspace index as JSON.
    #[arg(long, required = true)]
    dump: bool,
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    #[arg(long)]
    no_cache: bool,
}

#[derive(Args)]
pub struct PolicyArgs {
    /// Run rules to calculate active and unused suppression states.
    #[arg(long)]
    evaluate: bool,
    /// Use this configuration for every file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(flatten)]
    selection: SelectionArgs,
}

#[derive(Args)]
pub struct RuleArgs {
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    code: Option<String>,
    /// Print documentation for every implemented rule.
    #[arg(long)]
    all: bool,
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Read a Claude Code PostToolUse event from stdin.
    ClaudeCode {
        #[arg(long, value_name = "PATH")]
        config: Option<PathBuf>,
        #[command(flatten)]
        selection: SelectionArgs,
    },
}

#[derive(Serialize)]
struct PolicyReport<'a> {
    configurations: &'a BTreeMap<String, Settings>,
    files: Vec<PolicyRecord<'a>>,
    errors: &'a [InputError],
}

fn render(evaluation: &Analysis, format: CheckFormat) -> Result<String, String> {
    match format {
        CheckFormat::Text => {
            let filenames: BTreeSet<_> = evaluation
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.filename.as_str())
                .collect();
            let sources =
                filenames
                    .into_iter()
                    .filter_map(|filename| {
                        evaluation.snapshot.index.file(filename).map(|file| {
                            (file.filename().to_owned(), file.document().source.clone())
                        })
                    })
                    .collect();
            Ok(render_text(&evaluation.diagnostics, &sources))
        }
        CheckFormat::Concise => Ok(render_concise(&evaluation.diagnostics)),
        CheckFormat::Json => {
            render_json(&evaluation.diagnostics).map_err(|error| error.to_string())
        }
        CheckFormat::Sarif => {
            render_sarif(&evaluation.diagnostics).map_err(|error| error.to_string())
        }
        CheckFormat::Github => {
            let root = &evaluation.snapshot.index.root;
            let diagnostics = evaluation
                .diagnostics
                .iter()
                .cloned()
                .map(|mut diagnostic| {
                    diagnostic.filename = github_filename(root, &diagnostic.filename);
                    for related in &mut diagnostic.related {
                        related.filename = github_filename(root, &related.filename);
                    }
                    diagnostic
                })
                .collect::<Vec<_>>();
            Ok(render_github(&diagnostics))
        }
    }
}

/// Resolve an annotation location relative to the workspace checkout, or absolutely.
fn github_filename(workspace_root: &Path, filename: &str) -> String {
    let path = workspace_root.join(filename);
    let repository = repository_root(workspace_root);
    let reported = if repository.join(".git").exists() {
        path.strip_prefix(&repository).unwrap_or(&path)
    } else {
        &path
    };
    reported.to_string_lossy().replace('\\', "/")
}

fn print_errors(snapshot: &Snapshot, github: bool) {
    for error in &snapshot.errors {
        let message = format!("{}: {}", error.filename, error.message);
        if github {
            print_escaped_log(&message);
        } else {
            eprintln!("{message}");
        }
    }
}

pub fn check(args: CheckArgs) -> Result<u8, String> {
    let cwd = current_dir()?;
    let replacement = args
        .stdin_filename
        .as_ref()
        .map(|_| read_stdin("Cannot read UTF-8 stdin"))
        .transpose()?;
    let mut evaluation = evaluate(&cwd, &args, replacement)?;
    if args.fix && evaluation.snapshot.errors.is_empty() && evaluation.has_fixes() {
        // Recheck actual edit plans because suppression fixes can depend on other files.
        let fresh = evaluate(&cwd, &args, None)?;
        if !fresh.snapshot.errors.is_empty() || !evaluation.same_inputs_and_diagnostics(&fresh) {
            evaluation.snapshot.errors.push(InputError {
                filename: ".".into(),
                message: "Workspace inputs changed during the check; rerun before applying fixes."
                    .into(),
            });
        } else {
            let (changed, errors) = apply_safe_fixes(&fresh);
            evaluation = if changed > 0 {
                evaluate(&cwd, &args, None)?
            } else {
                fresh
            };
            evaluation.snapshot.errors.extend(errors);
            evaluation.snapshot.sort_errors();
        }
    }
    write_stdout(
        &render_evaluation(&evaluation, args.output_format, args.statistics)?,
        "Cannot write output",
    )?;
    if args.statistics && matches!(args.output_format, CheckFormat::Github) {
        print_escaped_log(&statistics_text(&evaluation));
    }
    let github = matches!(args.output_format, CheckFormat::Github);
    print_errors(&evaluation.snapshot, github);
    for notice in inactive_preview_selectors(&evaluation.snapshot, &args.selection) {
        print_notice(&notice, github);
    }
    for skipped in &evaluation.snapshot.skipped {
        if skipped.filename == "." {
            print_notice(&skipped.message, github);
        } else {
            print_notice(
                &format!("{}: {}", skipped.filename, skipped.message),
                github,
            );
        }
    }
    if !evaluation.snapshot.selected.is_empty()
        && evaluation.snapshot.enabled_count() == 0
        && evaluation.diagnostics.is_empty()
        && evaluation.snapshot.errors.is_empty()
    {
        print_notice(
            "No rules enabled for the selected files. Inspect `seiso policy` for the effective selection and file kinds.",
            github,
        );
    }
    Ok(evaluation.exit_code(args.exit_zero))
}

fn print_notice(message: &str, github: bool) {
    if github {
        print_escaped_log(message);
    } else {
        eprintln!("seiso: {message}");
    }
}

/// Name command-line selectors that cannot take effect because preview is off everywhere.
fn inactive_preview_selectors(snapshot: &Snapshot, selection: &SelectionArgs) -> Vec<String> {
    if selection.preview
        || snapshot
            .configurations
            .values()
            .any(|settings| settings.preview)
    {
        return Vec::new();
    }
    selection
        .select
        .iter()
        .flatten()
        .chain(&selection.extend_select)
        .filter(|selector| {
            // Codes and families are prefixes; `ALL` matches no prefix and stays silent.
            let mut matched = seiso::rules::rules()
                .iter()
                .filter(|rule| rule.code.starts_with(selector.as_str()))
                .peekable();
            matched.peek().is_some() && matched.all(|rule| !rule.is_stable())
        })
        .map(|selector| {
            format!("{selector} selects only preview rules, which are not enabled; add --preview or set `preview = true`.")
        })
        .collect()
}

pub fn policy(args: PolicyArgs) -> Result<u8, String> {
    let options = LoadOptions {
        config: args.config,
        overrides: args.selection.overrides(),
        no_cache: true,
        ..LoadOptions::default()
    };
    let mut snapshot = workspace::load(&current_dir()?, &options, LoadScope::Workspace)?;
    if args.evaluate {
        snapshot = analysis::check(snapshot).snapshot;
    } else {
        analysis::inspect_policy(&mut snapshot);
    }
    let report = PolicyReport {
        configurations: &snapshot.configurations,
        files: snapshot.policy_records(),
        errors: &snapshot.errors,
    };
    write_stdout(
        &(serde_json::to_string_pretty(&report).map_err(|error| error.to_string())? + "\n"),
        "Cannot write output",
    )?;
    print_errors(&snapshot, false);
    Ok(if snapshot.errors.is_empty() { 0 } else { 2 })
}

pub fn index(args: IndexArgs) -> Result<u8, String> {
    let options = LoadOptions {
        config: args.config,
        no_cache: args.no_cache,
        ..LoadOptions::default()
    };
    let snapshot = workspace::load(&current_dir()?, &options, LoadScope::Workspace)?;
    let result = serde_json::json!({"index":snapshot.index.dump(),"errors":snapshot.errors});
    write_stdout(
        &(serde_json::to_string_pretty(&result).map_err(|error| error.to_string())? + "\n"),
        "Cannot write output",
    )?;
    print_errors(&snapshot, false);
    Ok(if snapshot.errors.is_empty() { 0 } else { 2 })
}

/// Excluded documents have no suppressions, so only indexed documents are reported.
fn reported_policies(evaluation: &Analysis) -> impl Iterator<Item = &FilePolicy> {
    evaluation
        .snapshot
        .index
        .files()
        .iter()
        .map(IndexedFile::policy)
        .filter(|file| {
            evaluation.snapshot.selected.contains(&file.filename)
                || evaluation
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.filename == file.filename)
        })
}

fn statistics(evaluation: &Analysis) -> serde_json::Value {
    let mut counts = BTreeMap::<&str, usize>::new();
    for diagnostic in &evaluation.diagnostics {
        *counts.entry(&diagnostic.code).or_default() += 1;
    }
    let suppressions: Vec<_> = reported_policies(evaluation)
        .flat_map(|file| {
            file.suppressions
                .iter()
                .map(|record| serde_json::json!({"filename":file.filename,"declaration":record}))
        })
        .collect();
    serde_json::json!({"rules":counts,"suppressions":suppressions})
}

fn statistics_text(evaluation: &Analysis) -> String {
    let stats = statistics(evaluation);
    let mut text = String::from("Rule counts:\n");
    for (code, count) in stats["rules"].as_object().into_iter().flatten() {
        text.push_str(&format!("  {code}: {count}\n"));
    }
    text.push_str("Suppressions:\n");
    for file in reported_policies(evaluation) {
        for record in &file.suppressions {
            let reason = record.reason.replace(['\r', '\n'], " ");
            text.push_str(&format!(
                "  {}: {} -- {} ({})\n",
                file.filename,
                serde_json::json!(record.codes),
                reason,
                serde_json::json!(record.states)
            ));
        }
    }
    text
}

pub(crate) fn print_escaped_log(text: &str) {
    // The runner also parses stderr, including legacy commands embedded within
    // a line. Prefix every line and break that legacy delimiter in log content.
    for line in text.lines() {
        eprintln!(
            "seiso: {}",
            line.replace('\r', "\\r").replace("##[", "## [")
        );
    }
}

fn render_evaluation(
    evaluation: &Analysis,
    format: CheckFormat,
    include_statistics: bool,
) -> Result<String, String> {
    if !include_statistics {
        return render(evaluation, format);
    }
    match format {
        CheckFormat::Json => {
            let report = serde_json::json!({
                "diagnostics": evaluation.diagnostics,
                "statistics": statistics(evaluation),
            });
            serde_json::to_string_pretty(&report)
                .map(|text| text + "\n")
                .map_err(|error| error.to_string())
        }
        CheckFormat::Sarif => {
            let mut sarif: serde_json::Value = serde_json::from_str(&render(evaluation, format)?)
                .map_err(|error| error.to_string())?;
            sarif["runs"][0]["properties"] =
                serde_json::json!({"statistics":statistics(evaluation)});
            serde_json::to_string_pretty(&sarif)
                .map(|text| text + "\n")
                .map_err(|error| error.to_string())
        }
        CheckFormat::Github => render(evaluation, format),
        _ => Ok(render(evaluation, format)? + &statistics_text(evaluation)),
    }
}

fn apply_safe_fixes(evaluation: &Analysis) -> (usize, Vec<InputError>) {
    let mut errors = Vec::new();
    let mut changed = 0;
    let index = &evaluation.snapshot.index;
    for file in index.files() {
        let diagnostics = evaluation
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.filename == file.filename())
            .cloned()
            .collect::<Vec<_>>();
        let outcome = write_safe_fixes(file, &diagnostics);
        if matches!(outcome, Ok(true)) {
            changed += 1;
        }
        if let Err(message) = outcome {
            errors.push(InputError {
                filename: file.filename().to_owned(),
                message,
            });
        }
    }
    (changed, errors)
}

fn write_safe_fixes(file: &IndexedFile, diagnostics: &[Diagnostic]) -> Result<bool, String> {
    let source = &file.document().source;
    let Some(updated) = seiso::rules::fixes::apply_fixes(source, diagnostics)? else {
        return Ok(false);
    };
    let path = file.path();
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.permissions().readonly()
    {
        return Err("Cannot apply fixes to a symlink, non-file, or read-only file.".into());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Missing parent directory.")?)
            .map_err(|e| e.to_string())?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|e| e.to_string())?;
    temporary
        .write_all(updated.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.flush().map_err(|e| e.to_string())?;
    if fs::read(path).map_err(|e| e.to_string())? != source.as_bytes() {
        return Err("Source changed before the fix could be written; rerun the check.".into());
    }
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(true)
}

fn evaluate(cwd: &Path, args: &CheckArgs, replacement: Option<String>) -> Result<Analysis, String> {
    let options = LoadOptions {
        paths: args.paths.clone(),
        config: args.config.clone(),
        overrides: args.selection.overrides(),
        stdin: args.stdin_filename.clone().zip(replacement),
        no_cache: args.no_cache,
    };
    let snapshot = workspace::load(cwd, &options, LoadScope::Check)?;
    Ok(analysis::check(snapshot))
}

pub fn rule(args: RuleArgs) -> Result<u8, String> {
    let docs = if args.all {
        let mut rules: Vec<_> = seiso::rules::rules().iter().collect();
        rules.sort_by_key(|rule| rule.code);
        rules
            .into_iter()
            .map(render_rule)
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        let code = args.code.as_deref().unwrap_or_default();
        let rule = seiso::rules::rule(&code.to_ascii_uppercase()).ok_or_else(|| {
            format!(
                "Rule {code:?} is not in seiso {}; run `seiso rule --all` to list this version's rules. Rules added in a newer release need an upgrade.",
                env!("CARGO_PKG_VERSION")
            )
        })?;
        render_rule(rule)
    };
    write_stdout(&(docs + "\n"), "Cannot write output")?;
    Ok(0)
}

fn render_rule(rule: &seiso::rules::Rule) -> String {
    let mut output = String::new();
    let mut status_added = false;
    let documentation = rule.standalone_documentation();
    // The frontmatter declares the page kind for seiso's own checks, not for readers.
    let mut lines = documentation.trim_end().lines().peekable();
    if lines.peek() == Some(&"---") {
        lines.next();
        lines.by_ref().find(|line| *line == "---");
        while lines.peek().is_some_and(|line| line.trim().is_empty()) {
            lines.next();
        }
    }
    for line in lines {
        output.push_str(line);
        output.push('\n');
        if !status_added && line.starts_with("# ") {
            output.push_str(&format!("\nStatus: {}.\n", rule.status()));
            status_added = true;
        }
    }
    output.trim_end().to_owned()
}

pub fn init() -> Result<u8, String> {
    let cwd = current_dir()?;
    let workspace = Workspace::discover(&cwd, None).map_err(|error| error.to_string())?;
    if let Some(path) = workspace.config.source {
        return Err(format!(
            "Configuration already exists at {}; edit that file instead.",
            path.display()
        ));
    }
    // Without a configuration, discovery stops at the repository root.
    let root = workspace.root.clone();
    let suggested = suggest_configuration(&workspace);
    let contents = suggested.render();
    let path = write_configuration(&root, &contents)?;
    let created = if root == seiso::paths::normalize(&cwd) {
        "seiso.toml".to_owned()
    } else {
        path.display().to_string()
    };
    let suggestions = if suggested.sites.is_empty() {
        "exclusions and kind mappings"
    } else {
        "exclusions, kind mappings, and site entries"
    };
    write_stdout(
        &format!(
            "Created {created}. Review the suggested {suggestions}, then run `seiso check`.\n"
        ),
        "Cannot write output",
    )?;
    Ok(0)
}

#[derive(Debug)]
struct SuggestedConfig {
    excludes: Vec<String>,
    kinds: Vec<(String, &'static str)>,
    sites: Vec<SiteSuggestion>,
}

fn suggest_configuration(workspace: &Workspace) -> SuggestedConfig {
    let root = &workspace.root;
    let mut excludes: Vec<String> = [
        ".github/ISSUE_TEMPLATE",
        ".github/DISCUSSION_TEMPLATE",
        ".github/PULL_REQUEST_TEMPLATE",
        "node_modules",
        "vendor",
        "third_party",
    ]
    .into_iter()
    .filter(|directory| root.join(directory).is_dir())
    .map(|directory| format!("{directory}/**"))
    .collect();
    excludes.extend(community_files(root, "PULL_REQUEST_TEMPLATE.md"));
    excludes.extend(community_files(root, "CODE_OF_CONDUCT.md"));
    let mut kinds: Vec<(String, &str)> = [
        ("**/README.md", "readme", root.join("README.md").is_file()),
        (
            "**/CHANGELOG.md",
            "changelog",
            root.join("CHANGELOG.md").is_file(),
        ),
        ("docs/guides/**", "howto", root.join("docs/guides").is_dir()),
        ("docs/howto/**", "howto", root.join("docs/howto").is_dir()),
        (
            "docs/reference/**",
            "reference",
            root.join("docs/reference").is_dir(),
        ),
        (
            "docs/runbooks/**",
            "runbook",
            root.join("docs/runbooks").is_dir(),
        ),
        ("docs/adr/**", "adr", root.join("docs/adr").is_dir()),
        ("docs/plans/**", "plan", root.join("docs/plans").is_dir()),
    ]
    .into_iter()
    .filter(|(_, _, exists)| *exists)
    .map(|(path, kind, _)| (path.to_owned(), kind))
    .collect();
    for name in ["CONTRIBUTING.md", "SECURITY.md", "SUPPORT.md"] {
        kinds.extend(
            community_files(root, name)
                .into_iter()
                .map(|path| (path, "howto")),
        );
    }
    kinds.extend(
        agent_kind_suggestions(workspace)
            .into_iter()
            .map(|path| (path.to_owned(), "agents")),
    );
    let sites = site_suggestions(root);
    SuggestedConfig {
        excludes,
        kinds,
        sites,
    }
}

/// Suggest agent mappings only for files an ordinary check would discover.
fn agent_kind_suggestions(workspace: &Workspace) -> Vec<&'static str> {
    let root = &workspace.root;
    let patterns = [
        "**/AGENTS.md",
        "**/CLAUDE.md",
        "**/SKILL.md",
        ".github/copilot-instructions.md",
    ];
    let mut found = [false; 4];
    for entry in workspace::walk_workspace(root).flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_markdown(path) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let candidate = if relative == ".github/copilot-instructions.md" {
            Some(3)
        } else {
            match relative.rsplit('/').next() {
                Some("AGENTS.md") => Some(0),
                Some("CLAUDE.md") => Some(1),
                Some("SKILL.md") => Some(2),
                _ => None,
            }
        };
        let Some(candidate) = candidate else {
            continue;
        };
        let Ok(config) = workspace.config_for(path) else {
            continue;
        };
        if config.source.as_deref() != workspace.config.source.as_deref()
            || config.directory.as_path() != workspace.config.directory.as_path()
        {
            continue;
        }
        if !config.includes(path) || config.excludes(path) {
            continue;
        }
        found[candidate] = true;
    }
    patterns
        .into_iter()
        .zip(found)
        .filter_map(|(pattern, exists)| exists.then_some(pattern))
        .collect()
}

impl SuggestedConfig {
    fn render(&self) -> String {
        let Self {
            excludes,
            kinds,
            sites,
        } = self;
        let quote = |value: &str| toml::Value::String(value.to_owned()).to_string();
        let mut contents = String::from("include = [\"**/*.md\", \"**/*.markdown\"]\n");
        if !excludes.is_empty() {
            contents.push_str(
                "# Templates, dependencies, and adopted texts are not project documentation.\nexclude = [\n",
            );
            for pattern in excludes {
                contents.push_str(&format!("  {},\n", quote(pattern)));
            }
            contents.push_str("]\n");
        }
        contents.push_str(
            "preview = false\n\n# Review these path mappings and declare other kinds in document frontmatter.\n",
        );
        for (path, kind) in kinds {
            contents.push_str(&format!(
                "\n[[kinds]]\npath = {}\nkind = \"{kind}\"\n",
                quote(path)
            ));
        }
        if !sites.is_empty() {
            contents.push_str(
                "\n# Links in documents that a site generator renders resolve as site routes; review these entries.\n",
            );
        }
        for site in sites {
            let path = if site.root == "." {
                "**".to_owned()
            } else {
                format!("{}/**", site.root)
            };
            contents.push_str(&format!(
                "\n[[sites]] # {}\npath = {}\nroot = {}\n",
                site.found,
                quote(&path),
                quote(&site.root)
            ));
            if let Some(public) = &site.public {
                contents.push_str(&format!("public = {}\n", quote(public)));
            }
        }
        contents
    }
}

fn write_configuration(root: &Path, contents: &str) -> Result<PathBuf, String> {
    let path = root.join("seiso.toml");
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
        builder
    };
    let mut temporary = builder.tempfile_in(root).map_err(|error| {
        format!(
            "Cannot create {}: {error}; preserve or edit the existing configuration.",
            path.display()
        )
    })?;
    temporary
        .write_all(contents.as_bytes())
        .and_then(|()| temporary.flush())
        .map_err(|error| format!("Cannot write {}: {error}", path.display()))?;
    temporary.persist_noclobber(&path).map_err(|error| {
        format!(
            "Cannot create {}: {}; preserve or edit the existing configuration.",
            path.display(),
            error.error
        )
    })?;
    Ok(path)
}

#[derive(Debug)]
struct SiteSuggestion {
    /// Generator and configuration file that suggested this site.
    found: String,
    root: String,
    public: Option<String>,
}

/// Find VitePress, Docusaurus, mdBook, and MkDocs projects at the repository
/// root or one directory below it, and the directories their pages come from.
fn site_suggestions(root: &Path) -> Vec<SiteSuggestion> {
    let mut projects = vec![String::new()];
    if let Ok(entries) = fs::read_dir(root) {
        let mut directories: Vec<_> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                !name.starts_with('.')
                    && !["node_modules", "vendor", "third_party", "target"].contains(&name.as_str())
            })
            .collect();
        directories.sort();
        projects.extend(directories);
    }
    let join = |base: &str, path: &str| {
        let path = seiso::paths::normalize(Path::new(base).join(path))
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_empty() {
            ".".to_owned()
        } else {
            path
        }
    };
    let setting = |source: &str, pattern: &str| {
        regex::Regex::new(pattern)
            .expect("valid generator setting pattern")
            .captures(source)
            .map(|capture| capture[1].trim().to_owned())
    };
    let mut sites = Vec::new();
    for project in projects {
        let directory = root.join(&project);
        let file = |name: &str| {
            let path = directory.join(name);
            path.is_file().then(|| {
                (
                    join(&project, name),
                    fs::read_to_string(&path).unwrap_or_default(),
                )
            })
        };
        let vitepress = ["js", "ts", "mjs", "mts", "cjs", "cts"]
            .into_iter()
            .find_map(|extension| file(&format!(".vitepress/config.{extension}")));
        let docusaurus = ["js", "ts", "mjs", "cjs"]
            .into_iter()
            .find_map(|extension| file(&format!("docusaurus.config.{extension}")));
        let found = if let Some((path, source)) = vitepress {
            let source_directory = setting(&source, r#"\bsrcDir\s*:\s*['"`]([^'"`]+)['"`]"#);
            let pages = join(&project, source_directory.as_deref().unwrap_or("."));
            Some((
                format!("VitePress: {path}"),
                pages.clone(),
                join(&pages, "public"),
            ))
        } else if let Some((path, _)) = docusaurus {
            Some((
                format!("Docusaurus: {path}"),
                join(&project, "."),
                join(&project, "static"),
            ))
        } else if let Some((path, source)) = file("book.toml") {
            let source_directory = toml::from_str::<toml::Value>(&source)
                .ok()
                .and_then(|book| book.get("book")?.get("src")?.as_str().map(str::to_owned));
            let pages = join(&project, source_directory.as_deref().unwrap_or("src"));
            Some((format!("mdBook: {path}"), pages, String::new()))
        } else if let Some((path, source)) = file("mkdocs.yml").or_else(|| file("mkdocs.yaml")) {
            let source_directory = setting(
                &source,
                r##"(?m)^docs_dir\s*:\s*['"]?([^'"#\r\n]+?)['"]?\s*(?:#.*)?$"##,
            );
            let pages = join(&project, source_directory.as_deref().unwrap_or("docs"));
            Some((format!("MkDocs: {path}"), pages, String::new()))
        } else {
            None
        };
        if let Some((found, pages, public)) = found
            && root.join(&pages).is_dir()
        {
            sites.push(SiteSuggestion {
                found,
                root: pages,
                public: (!public.is_empty() && root.join(&public).is_dir()).then_some(public),
            });
        }
    }
    sites
}

/// Files GitHub reads from the repository root, `.github/`, or `docs/`, matched case-insensitively.
fn community_files(root: &Path, name: &str) -> Vec<String> {
    ["", ".github", "docs"]
        .into_iter()
        .filter_map(|directory| {
            fs::read_dir(root.join(directory))
                .ok()?
                .flatten()
                .find_map(|entry| {
                    let file_name = entry.file_name().into_string().ok()?;
                    (file_name.eq_ignore_ascii_case(name)
                        && entry.file_type().is_ok_and(|kind| kind.is_file()))
                    .then(|| {
                        if directory.is_empty() {
                            file_name
                        } else {
                            format!("{directory}/{file_name}")
                        }
                    })
                })
        })
        .collect()
}

pub fn hook(command: HookCommand) -> u8 {
    match run_hook(command) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("seiso: {error}");
            1
        }
    }
}

fn run_hook(command: HookCommand) -> Result<u8, String> {
    let HookCommand::ClaudeCode { config, selection } = command;
    let input = read_stdin("Cannot read UTF-8 stdin")?;
    let input: serde_json::Value = serde_json::from_str(&input).map_err(|error| {
        format!(
            "Cannot read Claude Code hook JSON: {error}; configure a PostToolUse Write|Edit hook."
        )
    })?;
    let cwd = input
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Claude Code hook input needs a non-empty cwd path.")?;
    let cwd = PathBuf::from(cwd);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("Claude Code hook cwd must be an existing absolute directory.".into());
    }
    let path = input.get("tool_input").and_then(|value| value.get("file_path"))
        .and_then(serde_json::Value::as_str).filter(|value| !value.is_empty())
        .ok_or("Claude Code hook input needs tool_input.file_path; use a PostToolUse Write|Edit matcher.")?;
    let path = PathBuf::from(path);
    if !is_markdown(&path) {
        return Ok(0);
    }
    let args = CheckArgs {
        paths: vec![path],
        config,
        selection,
        output_format: CheckFormat::Concise,
        stdin_filename: None,
        exit_zero: false,
        no_cache: false,
        fix: false,
        statistics: false,
    };
    let evaluation = evaluate(&cwd, &args, None)?;
    let code = evaluation.exit_code(false);
    if code == 0 {
        return Ok(0);
    }
    io::stderr()
        .lock()
        .write_all(render(&evaluation, CheckFormat::Concise)?.as_bytes())
        .map_err(|error| format!("Cannot write hook diagnostics: {error}"))?;
    if code == 1 {
        Ok(2)
    } else {
        print_errors(&evaluation.snapshot, false);
        Ok(1)
    }
}

fn current_dir() -> Result<PathBuf, String> {
    std::env::current_dir()
        .map_err(|error| format!("Cannot determine the current directory: {error}"))
}

fn read_stdin(description: &str) -> Result<String, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("{description}: {error}"))?;
    Ok(input)
}

fn write_stdout(text: &str, description: &str) -> Result<(), String> {
    io::stdout()
        .lock()
        .write_all(text.as_bytes())
        .map_err(|error| format!("{description}: {error}"))
}

#[derive(clap::Args)]
pub struct ParseArgs {
    /// Files or directories to inspect; defaults to the workspace.
    paths: Vec<PathBuf>,
    /// Use this configuration for every selected file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Output format. JSON contains the document model and heuristic section annotations.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    output_format: OutputFormat,
    /// Read stdin in place of this workspace file; never writes to disk.
    #[arg(long, value_name = "PATH", conflicts_with = "paths")]
    stdin_filename: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Serialize)]
struct ParsedFile {
    filename: String,
    configuration: Option<String>,
    kind: KindResolution,
    domain: Option<String>,
    section_annotations: Vec<seiso::sections::SectionAnnotation>,
    document: Document,
}

#[derive(Default, Serialize)]
struct ParseReport {
    files: Vec<ParsedFile>,
    errors: Vec<InputError>,
}

pub fn parse(args: ParseArgs) -> Result<u8, String> {
    let cwd = current_dir()?;
    let stdin = args
        .stdin_filename
        .map(|path| {
            read_stdin("Cannot read UTF-8 Markdown from stdin").map(|source| (path, source))
        })
        .transpose()?;
    let snapshot = workspace::load(
        &cwd,
        &LoadOptions {
            paths: args.paths,
            config: args.config,
            stdin,
            no_cache: true,
            ..LoadOptions::default()
        },
        LoadScope::Selected,
    )?;
    let root = snapshot.index.root.clone();
    let report = ParseReport {
        files: snapshot
            .index
            .into_files()
            .into_iter()
            .map(|file| ParsedFile {
                filename: file.filename().to_owned(),
                configuration: workspace::configuration_source(&root, file.config()),
                kind: file.policy().kind.resolution(),
                domain: file.policy().domain.clone(),
                section_annotations: seiso::sections::classify(file.document()),
                document: Arc::unwrap_or_clone(file.into_document()),
            })
            .collect(),
        errors: snapshot.errors,
    };
    let rendered = match args.output_format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("Cannot encode the parse report: {error}"))?
                + "\n"
        }
        OutputFormat::Text => render_summary(&report),
    };
    write_stdout(&rendered, "Cannot write the parse report")?;
    for error in &report.errors {
        eprintln!("{}: {}", error.filename, error.message);
    }
    Ok(if report.errors.is_empty() { 0 } else { 2 })
}

fn render_summary(report: &ParseReport) -> String {
    use std::fmt::Write as _;
    let mut output = String::new();
    for file in &report.files {
        let _ = writeln!(
            output,
            "{}: kind={} ({}), sections={}, blocks={}, sentences={}",
            file.filename,
            file.kind
                .value
                .map(seiso::rules::Kind::as_str)
                .unwrap_or("unknown"),
            file.kind.source,
            file.document.sections.len().saturating_sub(1),
            file.document.blocks.len(),
            file.document.sentences.len()
        );
        if let Some(problem) = &file.kind.problem {
            let _ = writeln!(output, "  {problem}");
        }
    }
    let _ = writeln!(
        output,
        "Parsed {} file(s). No lint rules were run.",
        report.files.len()
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_suggestions_are_values_before_rendering_or_writing() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("docs/guides")).unwrap();
        fs::create_dir_all(root.join(".github/ISSUE_TEMPLATE")).unwrap();
        fs::write(root.join("README.md"), "# Project").unwrap();
        fs::write(root.join("CONTRIBUTING.md"), "# Contribute").unwrap();
        fs::write(root.join("mkdocs.yml"), "site_name: Project\n").unwrap();
        let workspace = Workspace::discover(root, None).unwrap();
        let suggested = suggest_configuration(&workspace);
        assert_eq!(suggested.excludes, [".github/ISSUE_TEMPLATE/**"]);
        assert_eq!(
            suggested.kinds,
            [
                ("**/README.md".into(), "readme"),
                ("docs/guides/**".into(), "howto"),
                ("CONTRIBUTING.md".into(), "howto"),
            ]
        );
        assert_eq!(suggested.sites.len(), 1);
        assert_eq!(suggested.sites[0].root, "docs");
        assert!(!root.join("seiso.toml").exists());
    }
}
