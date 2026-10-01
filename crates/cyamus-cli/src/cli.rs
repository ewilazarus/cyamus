use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Opinionated worktree environment lifecycle for the Orca ADE.
#[derive(Debug, Parser)]
#[command(name = "cyamus", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set up or tear down a workspace (a worktree with cyamus goodies applied).
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Open the project's config directory in $EDITOR.
    Edit {
        /// Any path inside a worktree of the project (default: current directory).
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Apply assets and run on_setup hooks. Idempotent; also serves as reset.
    Setup(LifecycleArgs),
    /// Run on_teardown hooks. Never deletes the worktree or its branch.
    Teardown(LifecycleArgs),
}

#[derive(Debug, Args)]
pub struct LifecycleArgs {
    /// Any path inside the worktree (default: current directory).
    pub path: Option<PathBuf>,

    /// Runtime variable exposed to hooks as CYAMUS_VAR_<KEY>. Repeatable.
    #[arg(long = "var", value_name = "KEY=VALUE", value_parser = parse_var)]
    pub vars: Vec<(String, String)>,
}

fn parse_var(raw: &str) -> Result<(String, String), String> {
    match raw.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => Ok((key.to_owned(), value.to_owned())),
        _ => Err(format!("expected KEY=VALUE, got {raw:?}")),
    }
}
