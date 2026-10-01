//! The `cyamus.toml` manifest: schema, parsing and validation.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::naming::env_var_suffix;
use crate::paths::is_contained;

/// Highest manifest version this build understands.
pub const SUPPORTED_VERSION: u32 = 1;

pub const DEFAULT_FINGERPRINT_LENGTH: i64 = 12;
pub const MAX_FINGERPRINT_LENGTH: i64 = 64;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default, rename = "link")]
    pub links: Vec<Asset>,
    #[serde(default, rename = "copy")]
    pub copies: Vec<CopyAsset>,
    #[serde(default, rename = "fingerprint")]
    pub fingerprints: Vec<Fingerprint>,
    #[serde(default)]
    pub hooks: Hooks,
}

/// A `[[link]]` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub source: String,
    pub target: String,
    #[serde(default, rename = "override")]
    pub override_tracked: bool,
}

/// A `[[copy]]` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyAsset {
    pub source: String,
    pub target: String,
    #[serde(default, rename = "override")]
    pub override_tracked: bool,
    #[serde(default = "default_true")]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub name: String,
    pub files: Vec<String>,
    #[serde(default = "default_fingerprint_length")]
    pub length: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    #[serde(default)]
    pub on_setup: Vec<HookGroup>,
    #[serde(default)]
    pub on_teardown: Vec<HookGroup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookGroup {
    pub commands: Vec<String>,
    #[serde(default)]
    pub conditions: Option<Conditions>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    pub if_branch_matches: Option<String>,
    pub if_branch_not_matches: Option<String>,
}

fn default_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn default_fingerprint_length() -> i64 {
    DEFAULT_FINGERPRINT_LENGTH
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid manifest {path}:\n{message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid manifest {path}:\n{}", render_problems(.problems))]
    Invalid {
        path: PathBuf,
        problems: Vec<String>,
    },
}

fn render_problems(problems: &[String]) -> String {
    problems
        .iter()
        .map(|p| format!("  - {p}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Manifest {
    /// Reads, parses and validates the manifest at `path`.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let content = fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&content, path)
    }

    /// Parses and validates manifest content; `path` is used in error messages.
    pub fn parse(content: &str, path: &Path) -> Result<Self, ManifestError> {
        let manifest: Manifest = toml::from_str(content).map_err(|e| ManifestError::Parse {
            path: path.to_owned(),
            message: e.to_string().trim_end().to_owned(),
        })?;
        let problems = manifest.problems();
        if problems.is_empty() {
            Ok(manifest)
        } else {
            Err(ManifestError::Invalid {
                path: path.to_owned(),
                problems,
            })
        }
    }

    /// Semantic checks that the schema alone cannot express.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.version == 0 || self.version > SUPPORTED_VERSION {
            problems.push(format!(
                "unsupported manifest version {} (this build supports version {SUPPORTED_VERSION})",
                self.version
            ));
        }

        collisions(
            "[vars] keys",
            "CYAMUS_VAR_",
            self.vars.keys(),
            &mut problems,
        );

        let assets = self
            .links
            .iter()
            .map(|a| ("link", &a.source, &a.target))
            .chain(self.copies.iter().map(|c| ("copy", &c.source, &c.target)));
        for (kind, source, target) in assets {
            if !is_contained(Path::new(source)) {
                problems.push(format!(
                    "[[{kind}]] source {source:?} must be a path inside assets/"
                ));
            }
            if !is_contained(Path::new(target)) {
                problems.push(format!(
                    "[[{kind}]] target {target:?} must be a path inside the worktree"
                ));
            }
        }

        for fp in &self.fingerprints {
            if fp.name.trim().is_empty() {
                problems.push("[[fingerprint]] name must not be empty".to_owned());
            }
            if !(1..=MAX_FINGERPRINT_LENGTH).contains(&fp.length) {
                problems.push(format!(
                    "[[fingerprint]] {:?} length {} must be between 1 and {MAX_FINGERPRINT_LENGTH}",
                    fp.name, fp.length
                ));
            }
            for file in &fp.files {
                if !is_contained(Path::new(file)) {
                    problems.push(format!(
                        "[[fingerprint]] {:?} file {file:?} must be a path inside the worktree",
                        fp.name
                    ));
                }
            }
        }
        collisions(
            "[[fingerprint]] names",
            "CYAMUS_FINGERPRINT_",
            self.fingerprints.iter().map(|f| &f.name),
            &mut problems,
        );

        let groups = [
            ("on_setup", &self.hooks.on_setup),
            ("on_teardown", &self.hooks.on_teardown),
        ];
        for (event, groups) in groups {
            for group in groups {
                let Some(conditions) = &group.conditions else {
                    continue;
                };
                let patterns = [
                    ("if_branch_matches", &conditions.if_branch_matches),
                    ("if_branch_not_matches", &conditions.if_branch_not_matches),
                ];
                for (key, pattern) in patterns {
                    if let Some(pattern) = pattern
                        && let Err(e) = globset::Glob::new(pattern)
                    {
                        problems.push(format!("[[hooks.{event}]] {key} {pattern:?}: {e}"));
                    }
                }
            }
        }

        problems
    }
}

/// Reports names that normalize to the same environment variable.
fn collisions<'a>(
    what: &str,
    prefix: &str,
    names: impl Iterator<Item = &'a String>,
    problems: &mut Vec<String>,
) {
    let mut by_var: HashMap<String, Vec<&str>> = HashMap::new();
    for name in names {
        by_var.entry(env_var_suffix(name)).or_default().push(name);
    }
    let mut clashes: Vec<_> = by_var.into_iter().filter(|(_, n)| n.len() > 1).collect();
    clashes.sort();
    for (var, names) in clashes {
        let names = names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(format!("{what} {names} all map to {prefix}{var}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Result<Manifest, ManifestError> {
        Manifest::parse(src, Path::new("cyamus.toml"))
    }

    fn err(src: &str) -> String {
        parse(src).unwrap_err().to_string()
    }

    #[test]
    fn empty_manifest_defaults() {
        let m = parse("").unwrap();
        assert_eq!(m.version, 1);
        assert!(m.vars.is_empty() && m.links.is_empty() && m.copies.is_empty());
        assert!(m.hooks.on_setup.is_empty() && m.hooks.on_teardown.is_empty());
    }

    #[test]
    fn full_manifest() {
        let m = parse(
            r#"
            version = 1
            [vars]
            node-version = "22"

            [[link]]
            source = "env.local"
            target = ".env.local"

            [[copy]]
            source = "config.yaml"
            target = "config.yaml"
            overwrite = false

            [[fingerprint]]
            name = "docker-deps"
            files = ["Dockerfile", "package.json"]

            [[hooks.on_setup]]
            commands = ["npm install"]

            [[hooks.on_setup]]
            conditions = { if_branch_matches = "gabriel/*" }
            commands = ["tmux new -d"]

            [[hooks.on_teardown]]
            commands = ["clean"]
            "#,
        )
        .unwrap();
        assert_eq!(m.vars["node-version"], "22");
        assert!(!m.links[0].override_tracked);
        assert!(!m.copies[0].overwrite && !m.copies[0].override_tracked);
        assert_eq!(m.fingerprints[0].length, 12);
        assert_eq!(m.hooks.on_setup.len(), 2);
        assert!(m.hooks.on_setup[0].conditions.is_none());
        assert_eq!(
            m.hooks.on_setup[1]
                .conditions
                .as_ref()
                .unwrap()
                .if_branch_matches
                .as_deref(),
            Some("gabriel/*")
        );
    }

    #[test]
    fn copy_overwrite_defaults_to_true() {
        let m = parse("[[copy]]\nsource = \"a\"\ntarget = \"b\"\n").unwrap();
        assert!(m.copies[0].overwrite);
    }

    #[test]
    fn missing_required_key() {
        let e = err("[[copy]]\nsource = \"a\"\n");
        assert!(e.contains("target"), "{e}");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let e = err("[[copy]]\nsource = \"a\"\ntarget = \"b\"\noverwite = false\n");
        assert!(e.contains("overwite"), "{e}");
        let e = err("[[hooks.on_attach]]\ncommands = []\n");
        assert!(e.contains("on_attach"), "{e}");
        let e = err("base_branch = \"main\"\n");
        assert!(e.contains("base_branch"), "{e}");
        let e = err("[prune]\nolder_than_days = 3\n");
        assert!(e.contains("prune"), "{e}");
        let e = err("[[hooks.on_setup]]\ncommands = []\nconditions = { if_branch = \"x\" }\n");
        assert!(e.contains("if_branch"), "{e}");
    }

    #[test]
    fn syntax_error_reports_path_and_line() {
        let e = err("version = 1\n\n[vars]\nbroken = \n");
        assert!(e.contains("cyamus.toml"), "{e}");
        assert!(e.contains("line 4"), "{e}");
    }

    #[test]
    fn unsupported_version() {
        let e = err("version = 2\n");
        assert!(e.contains("unsupported manifest version 2"), "{e}");
        assert!(e.contains("supports version 1"), "{e}");
    }

    #[test]
    fn colliding_vars() {
        let e = err("[vars]\nnode-version = \"1\"\nnode_version = \"2\"\n");
        assert!(
            e.contains("node-version") && e.contains("node_version"),
            "{e}"
        );
        assert!(e.contains("CYAMUS_VAR_NODE_VERSION"), "{e}");
    }

    #[test]
    fn escaping_asset_paths() {
        let e = err("[[link]]\nsource = \"a\"\ntarget = \"../../etc/foo\"\n");
        assert!(e.contains("../../etc/foo"), "{e}");
        let e = err("[[copy]]\nsource = \"/etc/passwd\"\ntarget = \"b\"\n");
        assert!(e.contains("/etc/passwd"), "{e}");
    }

    #[test]
    fn invalid_fingerprints() {
        let e = err("[[fingerprint]]\nname = \" \"\nfiles = []\n");
        assert!(e.contains("name must not be empty"), "{e}");
        let e = err("[[fingerprint]]\nname = \"x\"\nfiles = []\nlength = 0\n");
        assert!(e.contains("length 0"), "{e}");
        let e = err("[[fingerprint]]\nname = \"x\"\nfiles = []\nlength = 65\n");
        assert!(e.contains("length 65"), "{e}");
        let e = err("[[fingerprint]]\nname = \"x\"\nfiles = [\"../secret\"]\n");
        assert!(e.contains("../secret"), "{e}");
    }

    #[test]
    fn colliding_fingerprints() {
        let e = err("[[fingerprint]]\nname = \"docker-deps\"\nfiles = []\n\
             [[fingerprint]]\nname = \"docker_deps\"\nfiles = []\n");
        assert!(
            e.contains("docker-deps") && e.contains("docker_deps"),
            "{e}"
        );
    }

    #[test]
    fn invalid_glob() {
        let e =
            err("[[hooks.on_setup]]\ncommands = []\nconditions = { if_branch_matches = \"[\" }\n");
        assert!(e.contains("if_branch_matches"), "{e}");
    }

    #[test]
    fn collects_all_problems() {
        let e = err("version = 9\n[[link]]\nsource = \"../a\"\ntarget = \"../b\"\n");
        assert_eq!(e.matches("\n  - ").count(), 3, "{e}");
    }
}
