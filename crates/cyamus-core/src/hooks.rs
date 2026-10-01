//! Selecting and running lifecycle hook commands.

use std::collections::BTreeMap;
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use globset::Glob;

use crate::manifest::{Conditions, HookGroup};
use crate::report::Reporter;

/// A lifecycle event that runs hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Setup,
    Teardown,
}

impl Event {
    /// Value of `CYAMUS_EVENT`.
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Setup => "setup",
            Event::Teardown => "teardown",
        }
    }

    /// Manifest key of the event's hook groups.
    pub fn hook_key(self) -> &'static str {
        match self {
            Event::Setup => "on_setup",
            Event::Teardown => "on_teardown",
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.hook_key())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("invalid branch pattern {pattern:?}")]
    Pattern {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    #[error("{event} hook {command:?}: bin script {} is not executable", .path.display())]
    NotExecutable {
        event: Event,
        command: String,
        path: PathBuf,
    },
    #[error("{event} hook {command:?} could not be started")]
    Spawn {
        event: Event,
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{event} hook {command:?} {}", describe_exit(*.code))]
    Failed {
        event: Event,
        command: String,
        code: Option<i32>,
    },
}

fn describe_exit(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exited with code {code}"),
        None => "was terminated by a signal".to_owned(),
    }
}

/// Returns the commands of every group whose conditions match `branch`, in
/// declaration order, skipping blank entries.
pub fn select<'a>(groups: &'a [HookGroup], branch: &str) -> Result<Vec<&'a str>, HookError> {
    let mut selected = Vec::new();
    for group in groups {
        if matches(group.conditions.as_ref(), branch)? {
            selected.extend(
                group
                    .commands
                    .iter()
                    .map(String::as_str)
                    .filter(|c| !c.trim().is_empty()),
            );
        }
    }
    Ok(selected)
}

fn matches(conditions: Option<&Conditions>, branch: &str) -> Result<bool, HookError> {
    let Some(conditions) = conditions else {
        return Ok(true);
    };
    let glob = |pattern: &str| {
        Glob::new(pattern)
            .map(|g| g.compile_matcher())
            .map_err(|source| HookError::Pattern {
                pattern: pattern.to_owned(),
                source,
            })
    };
    if let Some(pattern) = &conditions.if_branch_matches
        && !glob(pattern)?.is_match(branch)
    {
        return Ok(false);
    }
    if let Some(pattern) = &conditions.if_branch_not_matches
        && glob(pattern)?.is_match(branch)
    {
        return Ok(false);
    }
    Ok(true)
}

/// Runs hook commands for one workspace.
pub struct HookRunner<'a> {
    pub worktree: &'a Path,
    pub bin_dir: &'a Path,
    pub env: &'a BTreeMap<String, String>,
}

impl HookRunner<'_> {
    /// Runs the matching commands of `groups` sequentially, stopping at the
    /// first failure.
    pub fn run(
        &self,
        event: Event,
        groups: &[HookGroup],
        branch: &str,
        reporter: &mut dyn Reporter,
    ) -> Result<(), HookError> {
        for command in select(groups, branch)? {
            reporter.hook_started(event, command);
            self.run_one(event, command)?;
        }
        Ok(())
    }

    fn run_one(&self, event: Event, entry: &str) -> Result<(), HookError> {
        let script = self.bin_dir.join(entry);
        let mut cmd = if script.is_file() {
            let mode = script
                .metadata()
                .map(|m| m.permissions().mode())
                .unwrap_or(0);
            if mode & 0o111 == 0 {
                return Err(HookError::NotExecutable {
                    event,
                    command: entry.to_owned(),
                    path: script,
                });
            }
            Command::new(&script)
        } else {
            let shell = std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "sh".into());
            let mut cmd = Command::new(shell);
            cmd.arg("-c").arg(entry);
            cmd
        };
        let status = cmd
            .current_dir(self.worktree)
            .envs(self.env)
            .env("CYAMUS_EVENT", event.as_str())
            .status()
            .map_err(|source| HookError::Spawn {
                event,
                command: entry.to_owned(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(HookError::Failed {
                event,
                command: entry.to_owned(),
                code: status.code(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(commands: &[&str], matches: Option<&str>, not_matches: Option<&str>) -> HookGroup {
        HookGroup {
            commands: commands.iter().map(|c| c.to_string()).collect(),
            conditions: (matches.is_some() || not_matches.is_some()).then(|| Conditions {
                if_branch_matches: matches.map(str::to_owned),
                if_branch_not_matches: not_matches.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn unconditional_groups_always_run() {
        let groups = [group(&["a", "b"], None, None)];
        assert_eq!(select(&groups, "anything").unwrap(), ["a", "b"]);
    }

    #[test]
    fn star_crosses_slashes() {
        let groups = [group(&["mine"], Some("gabriel/*"), None)];
        assert_eq!(select(&groups, "gabriel/feat/x").unwrap(), ["mine"]);
        assert!(select(&groups, "other/x").unwrap().is_empty());
    }

    #[test]
    fn not_matches_excludes() {
        let groups = [group(&["theirs"], None, Some("gabriel/*"))];
        assert!(select(&groups, "gabriel/x").unwrap().is_empty());
        assert_eq!(select(&groups, "main").unwrap(), ["theirs"]);
    }

    #[test]
    fn conditions_are_anded() {
        let groups = [group(&["x"], Some("gabriel/*"), Some("gabriel/wip-*"))];
        assert!(select(&groups, "gabriel/wip-1").unwrap().is_empty());
        assert_eq!(select(&groups, "gabriel/feat").unwrap(), ["x"]);
    }

    #[test]
    fn question_mark_and_classes() {
        let groups = [group(&["x"], Some("v?.[0-9]"), None)];
        assert_eq!(select(&groups, "v1.2").unwrap(), ["x"]);
        assert!(select(&groups, "v1.a").unwrap().is_empty());
    }

    #[test]
    fn preserves_order_and_skips_blank_entries() {
        let groups = [
            group(&["1", " "], None, None),
            group(&["skip"], Some("nope"), None),
            group(&["2", ""], None, None),
        ];
        assert_eq!(select(&groups, "main").unwrap(), ["1", "2"]);
    }
}
