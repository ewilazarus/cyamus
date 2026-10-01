mod cli;
mod edit;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use cyamus_core::hooks::Event;
use cyamus_core::lifecycle;
use cyamus_core::paths::Dirs;
use cyamus_core::report::Reporter;

use crate::cli::{Cli, Command, LifecycleArgs, WorkspaceCommand};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("cyamus: error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let mut reporter = StderrReporter;
    match cli.command {
        Command::Workspace { command } => {
            let dirs = Dirs::from_env()?;
            match command {
                WorkspaceCommand::Setup(args) => {
                    let path = resolve_path(&args)?;
                    lifecycle::setup(&path, &args.vars, &dirs, &mut reporter)?;
                }
                WorkspaceCommand::Teardown(args) => {
                    let path = resolve_path(&args)?;
                    lifecycle::teardown(&path, &args.vars, &dirs, &mut reporter)?;
                }
            }
            Ok(())
        }
        Command::Edit { path } => edit::run(path, &mut reporter),
    }
}

fn resolve_path(args: &LifecycleArgs) -> anyhow::Result<PathBuf> {
    match &args.path {
        Some(path) => Ok(path.clone()),
        None => std::env::current_dir().context("cannot determine the current directory"),
    }
}

/// Writes progress to stderr so stdout stays free for hook output.
struct StderrReporter;

impl Reporter for StderrReporter {
    fn notice(&mut self, message: &str) {
        eprintln!("cyamus: {message}");
    }

    fn hook_started(&mut self, event: Event, command: &str) {
        let _ = std::io::stdout().flush();
        eprintln!("▸ {event}: {command}");
    }
}
