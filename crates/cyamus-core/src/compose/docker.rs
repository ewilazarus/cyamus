//! The `docker` CLI calls cyamus makes around compose. Kept small and
//! synchronous; core never links a Docker client.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Labels cyamus puts on the networks it creates.
pub const PROJECT_LABEL: &str = "dev.cyamus.project";
pub const WORKSPACE_LABEL: &str = "dev.cyamus.workspace";

/// Oldest Compose release that understands the `!override` tag.
pub const MIN_COMPOSE: (u32, u32, u32) = (2, 24, 4);

/// How to run `docker` for one stack: environment changes and directory.
#[derive(Debug, Clone, Default)]
pub struct Docker {
    pub set: BTreeMap<String, String>,
    pub remove: Vec<String>,
    pub cwd: PathBuf,
}

impl Docker {
    pub fn command(&self) -> Command {
        let mut cmd = Command::new("docker");
        for key in &self.remove {
            cmd.env_remove(key);
        }
        cmd.envs(&self.set).current_dir(&self.cwd);
        cmd
    }

    fn output(&self, args: &[&str]) -> Result<Output, String> {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run docker: {e}"))
    }

    fn checked(&self, args: &[&str]) -> Result<String, String> {
        let out = self.output(args)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!(
                "`docker {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    /// `docker compose -p <project> <files> --profile '*' config --format json`
    pub fn config_json(&self, project: &str, files: &[OsString]) -> Result<String, String> {
        let mut cmd = self.command();
        cmd.args(["compose", "-p", project]);
        for f in files {
            cmd.arg("-f").arg(f);
        }
        let out = cmd
            .args(["--profile", "*", "config", "--format", "json"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run docker: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!(
                "cannot read the compose configuration: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }

    /// Services of a project that have a running container.
    pub fn running_services(&self, project: &str) -> Result<Vec<String>, String> {
        Ok(self
            .checked(&[
                "compose",
                "-p",
                project,
                "ps",
                "--status",
                "running",
                "--services",
            ])?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// Running containers of a compose project, as `(id, service)`.
    pub fn project_containers(&self, project: &str) -> Result<Vec<(String, String)>, String> {
        let filter = format!("label=com.docker.compose.project={project}");
        let format = r#"{{.ID}} {{.Label "com.docker.compose.service"}}"#;
        Ok(self
            .checked(&["ps", "--filter", &filter, "--format", format])?
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(id, svc)| (id.to_owned(), svc.to_owned()))
            .collect())
    }

    /// Creates `name` unless it exists; a concurrent creation is fine.
    pub fn ensure_network(&self, name: &str, labels: &[(&str, &str)]) -> Result<(), String> {
        if self.output(&["network", "inspect", name])?.status.success() {
            return Ok(());
        }
        let labels: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut args = vec!["network", "create"];
        for l in &labels {
            args.extend(["--label", l]);
        }
        args.push(name);
        match self.checked(&args) {
            Ok(_) => Ok(()),
            // Lost a race with another cyamus: fine if it exists now.
            Err(e) if self.output(&["network", "inspect", name])?.status.success() => {
                let _ = e;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Networks carrying `label=value`.
    pub fn labelled_networks(&self, label: &str, value: &str) -> Result<Vec<String>, String> {
        let filter = format!("label={label}={value}");
        Ok(self
            .checked(&[
                "network",
                "ls",
                "--filter",
                &filter,
                "--format",
                "{{.Name}}",
            ])?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// Container IDs (full) attached to `network`.
    pub fn attached(&self, network: &str) -> Result<Vec<String>, String> {
        Ok(self
            .checked(&[
                "network",
                "inspect",
                network,
                "--format",
                "{{range $id, $c := .Containers}}{{$id}} {{end}}",
            ])?
            .split_whitespace()
            .map(str::to_owned)
            .collect())
    }

    /// Connects each `(id, service)` to `network` under its service name,
    /// skipping containers already on it.
    pub fn attach(&self, network: &str, containers: &[(String, String)]) -> Result<(), String> {
        let attached = self.attached(network)?;
        for (id, service) in containers {
            if attached.iter().any(|a| a.starts_with(id.as_str())) {
                continue;
            }
            self.checked(&["network", "connect", "--alias", service, network, id])?;
        }
        Ok(())
    }

    /// Disconnects every container from `network`, then removes it.
    pub fn remove_network(&self, network: &str) -> Result<(), String> {
        for id in self.attached(network)? {
            self.checked(&["network", "disconnect", "--force", network, &id])?;
        }
        self.checked(&["network", "rm", network]).map(|_| ())
    }

    /// `docker compose version --short` as a version triple.
    pub fn compose_version(&self) -> Result<(u32, u32, u32), String> {
        let raw = self
            .checked(&["compose", "version", "--short"])
            .map_err(|_| "Docker Compose v2 is required (`docker compose` failed)".to_owned())?;
        parse_version(raw.trim()).ok_or_else(|| format!("cannot parse Compose version {raw:?}"))
    }
}

pub fn parse_version(raw: &str) -> Option<(u32, u32, u32)> {
    let core = raw.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()?.unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    ))
}

pub fn check_version(found: (u32, u32, u32)) -> Result<(), String> {
    if found >= MIN_COMPOSE {
        Ok(())
    } else {
        let (a, b, c) = MIN_COMPOSE;
        let (x, y, z) = found;
        Err(format!(
            "Docker Compose {a}.{b}.{c} or later is required (found {x}.{y}.{z})"
        ))
    }
}

/// `path` relative to `cwd` when given relatively, as compose resolves `-f`.
pub fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("5.1.2"), Some((5, 1, 2)));
        assert_eq!(parse_version("v2.24.4-desktop.1"), Some((2, 24, 4)));
        assert_eq!(parse_version("2.30"), Some((2, 30, 0)));
        assert_eq!(parse_version("garbage"), None);
        assert!(check_version((2, 24, 4)).is_ok());
        assert!(check_version((5, 1, 2)).is_ok());
        let err = check_version((2, 20, 2)).unwrap_err();
        assert!(
            err.contains("2.24.4 or later") && err.contains("2.20.2"),
            "{err}"
        );
    }
}
