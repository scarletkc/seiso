use clap::{Parser, Subcommand};
use std::process::ExitCode;

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
    /// Create a repository-root configuration with suggested exclusions and kind mappings.
    Init,
    /// Adapt editor events to Markdown checks.
    Hook {
        #[command(subcommand)]
        command: commands::HookCommand,
    },
    /// Parse Markdown into the seiso document model (does not run lint rules).
    Parse(commands::ParseArgs),
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            commands::print_escaped_log(&error);
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<u8, String> {
    match cli.command {
        Command::Parse(args) => commands::parse(args),
        Command::Check(args) => commands::check(args),
        Command::Policy(args) => commands::policy(args),
        Command::Index(args) => commands::index(args),
        Command::Rule(args) => commands::rule(args),
        Command::Init => commands::init(),
        Command::Hook { command } => Ok(commands::hook(command)),
    }
}
