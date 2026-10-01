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
    /// Run, inspect or stop the routing daemon (http://<service>.<branch>.<project>.localhost).
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Run docker compose for this worktree's stack (shared services come from compose-shared).
    #[command(disable_help_flag = true)]
    Compose {
        /// Arguments for `docker compose`, e.g. `up -d`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run docker compose for the project's shared stack (<config>/compose.yaml).
    #[command(name = "compose-shared", disable_help_flag = true)]
    ComposeShared {
        /// Arguments for `docker compose`, e.g. `up -d`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Run the daemon in the foreground (setup normally starts it for you).
    Run {
        /// Detach from the terminal's session; used when setup starts the daemon.
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Show whether the daemon runs, and its routes. Exits 1 when it isn't running.
    Status,
    /// Stop the running daemon.
    Stop,
    /// Forward loopback port 80 to the daemon; run by the service `install` sets up.
    #[command(hide = true)]
    Relay {
        /// Daemon port to forward to.
        #[arg(long)]
        to: u16,
        /// Port to accept connections on.
        #[arg(long, default_value_t = 80)]
        listen: u16,
        /// When started as root: serve from a child running as this uid.
        #[arg(long, value_name = "UID")]
        user: Option<u32>,
        /// Serve the sockets passed as stdin (IPv4) and stdout (IPv6).
        #[arg(long, hide = true)]
        inherited: bool,
    },
    /// Install the port-80 relay so URLs need no port (uses sudo once).
    Install {
        /// Print the files and commands without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the port-80 relay that `install` set up (uses sudo).
    Uninstall {
        /// Print the commands without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
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
