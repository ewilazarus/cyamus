//! `cyamus edit`: open the project config directory in `$EDITOR`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, bail};
use cyamus_core::paths::Dirs;
use cyamus_core::project::Project;
use cyamus_core::report::Reporter;

pub fn run(path: Option<PathBuf>, reporter: &mut dyn Reporter) -> anyhow::Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_default();
    if editor.trim().is_empty() {
        bail!("EDITOR is not set; set it to your editor command (e.g. `export EDITOR=nvim`)");
    }

    let path = match path {
        Some(path) => path,
        None => std::env::current_dir().context("cannot determine the current directory")?,
    };
    let dirs = Dirs::from_env()?;
    let project = Project::locate(&path, &dirs, reporter)?;
    if !project.is_configured() {
        reporter.notice(&format!(
            "creating config directory for project {:?} at {}",
            project.name,
            project.paths.root.display()
        ));
    }
    project.scaffold()?;

    // Run through the shell so EDITOR may carry arguments (e.g. `code -w`).
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&project.paths.root)
        .status()
        .with_context(|| format!("failed to launch editor {editor:?}"))?;
    if !status.success() {
        bail!("editor {editor:?} exited with {status}");
    }
    Ok(())
}
