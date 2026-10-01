mod cli;
mod daemon;
mod edit;
mod redirect;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use cyamus_core::hooks::Event;
use cyamus_core::lifecycle;
use cyamus_core::paths::Dirs;
use cyamus_core::report::Reporter;

use crate::cli::{Cli, Command, DaemonCommand, LifecycleArgs, WorkspaceCommand};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("cyamus: error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let mut reporter = StderrReporter;
    match cli.command {
        Command::Workspace { command } => {
            let dirs = Dirs::from_env()?;
            let port = daemon::port()?;
            let mut control = daemon::Spawner { dirs: &dirs, port };
            match command {
                WorkspaceCommand::Setup(args) => {
                    let path = resolve_path(&args)?;
                    lifecycle::setup(&path, &args.vars, &dirs, &mut control, &mut reporter)?;
                }
                WorkspaceCommand::Teardown(args) => {
                    let path = resolve_path(&args)?;
                    lifecycle::teardown(&path, &args.vars, &dirs, &control, &mut reporter)?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        // The relay runs from launchd/systemd without HOME; it needs no dirs.
        Command::Daemon {
            command:
                DaemonCommand::Relay {
                    to,
                    listen,
                    user,
                    inherited,
                },
        } => daemon::relay(to, listen, user, inherited),
        Command::Daemon { command } => {
            let dirs = Dirs::from_env()?;
            let port = daemon::port()?;
            match command {
                DaemonCommand::Run { background } => daemon::run(&dirs, port, background),
                DaemonCommand::Status => daemon::status(&dirs, port),
                DaemonCommand::Stop => daemon::stop(&dirs, port),
                DaemonCommand::Install { dry_run } => daemon::install(&dirs, port, dry_run),
                DaemonCommand::Uninstall { dry_run } => daemon::uninstall(port, dry_run),
                DaemonCommand::Relay { .. } => unreachable!("handled above"),
            }
        }
        Command::Edit { path } => edit::run(path, &mut reporter).map(|()| ExitCode::SUCCESS),
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
