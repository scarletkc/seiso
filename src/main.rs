use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use seiso::md::Document;
use seiso::rules::{KindResolution, resolve_kind};
use seiso::workspace::{self, InputError, LoadOptions, LoadScope};
use serde::Serialize;

mod commands;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "seiso",
    version,
    about = "A Markdown convention and linter for project docs written by AI and read by humans and agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check Markdown and report documentation diagnostics.
    Check(commands::CheckArgs),
    /// Print the effective file policies as deterministic JSON.
    Policy(commands::PolicyArgs),
    /// Inspect headings, links, and file roles in the current workspace index.
    Index(commands::IndexArgs),
    /// Print a rule's explanation and examples.
    Rule(commands::RuleArgs),
    /// Create a repository-root configuration or a child overlay with --extend.
    Init {
        /// Extend the nearest governing ancestor without replacing its policy lists.
        #[arg(long)]
        extend: bool,
    },
    /// Adapt editor events to Markdown checks.
    Hook {
        #[command(subcommand)]
        command: commands::HookCommand,
    },
    /// Parse Markdown into the seiso document model (does not run lint rules).
    Parse(ParseArgs),
}

#[derive(clap::Args)]
struct ParseArgs {
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

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            for line in error.lines() {
                eprintln!(
                    "seiso: {}",
                    line.replace('\r', "\\r").replace("##[", "## [")
                );
            }
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<u8, String> {
    match cli.command {
        Command::Parse(args) => parse_workspace(args),
        Command::Check(args) => commands::check(args),
        Command::Policy(args) => commands::policy(args),
        Command::Index(args) => commands::index(args),
        Command::Rule(args) => commands::rule(args),
        Command::Init { extend } => commands::init(extend),
        Command::Hook { command } => Ok(commands::hook(command)),
    }
}

fn parse_workspace(args: ParseArgs) -> Result<u8, String> {
    let cwd = std::env::current_dir()
        .map_err(|error| format!("Cannot determine the current directory: {error}"))?;
    let stdin = args
        .stdin_filename
        .map(|path| {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|error| format!("Cannot read UTF-8 Markdown from stdin: {error}"))?;
            Ok::<_, String>((path, source))
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
    let root = snapshot.selection_root.clone();
    let labels: Vec<_> = snapshot
        .index
        .files()
        .iter()
        .map(|file| snapshot.display_name(&file.filename))
        .collect();
    let errors = snapshot
        .errors
        .iter()
        .cloned()
        .map(|mut error| {
            if error.filename != "." {
                error.filename = snapshot.display_name(&error.filename);
            }
            error
        })
        .collect();
    let report = ParseReport {
        files: snapshot
            .index
            .into_files()
            .into_iter()
            .zip(labels)
            .map(|(file, label)| ParsedFile {
                filename: label,
                configuration: file
                    .config
                    .source
                    .as_ref()
                    .map(|path| workspace::relative(&root, path)),
                kind: resolve_kind(&file.document, file.config.kind_for(&file.path)),
                domain: file.config.domain_for(&file.path).map(str::to_owned),
                section_annotations: seiso::sections::classify(&file.document),
                document: Arc::unwrap_or_clone(file.document),
            })
            .collect(),
        errors,
    };
    let rendered = match args.output_format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("Cannot encode the parse report: {error}"))?
                + "\n"
        }
        OutputFormat::Text => render_summary(&report),
    };
    io::stdout()
        .lock()
        .write_all(rendered.as_bytes())
        .map_err(|error| format!("Cannot write the parse report: {error}"))?;
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
            file.kind.value.as_deref().unwrap_or("unknown"),
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
