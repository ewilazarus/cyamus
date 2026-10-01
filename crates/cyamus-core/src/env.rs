//! The `CYAMUS_*` environment exposed to hooks.

use std::collections::BTreeMap;

use crate::naming::{env_var_suffix, slugify};
use crate::workspace::Workspace;

pub const VAR_PREFIX: &str = "CYAMUS_VAR_";
pub const FINGERPRINT_PREFIX: &str = "CYAMUS_FINGERPRINT_";

/// Builds the `CYAMUS_*` variables for a workspace (everything except
/// `CYAMUS_EVENT`, which the hook runner sets per event).
///
/// Runtime variables override manifest variables with the same normalized
/// name. Hooks inherit the parent process environment on top of these.
pub fn build(
    workspace: &Workspace,
    manifest_vars: &BTreeMap<String, String>,
    runtime_vars: &[(String, String)],
    fingerprints: &[(String, String)],
) -> BTreeMap<String, String> {
    let project = &workspace.project;
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let mut env = BTreeMap::from([
        ("CYAMUS_PROJECT".to_owned(), project.name.clone()),
        ("CYAMUS_BRANCH".to_owned(), workspace.branch.clone()),
        (
            "CYAMUS_BRANCH_SLUG".to_owned(),
            slugify(&workspace.branch, '-'),
        ),
        (
            "CYAMUS_BRANCH_SNAKE".to_owned(),
            slugify(&workspace.branch, '_'),
        ),
        ("CYAMUS_WORKSPACE".to_owned(), path(&workspace.root)),
        ("CYAMUS_CONFIG_DIR".to_owned(), path(&project.paths.root)),
        ("CYAMUS_ASSETS".to_owned(), path(&project.paths.assets())),
        ("CYAMUS_BIN".to_owned(), path(&project.paths.bin())),
        ("CYAMUS_CACHE_DIR".to_owned(), path(&project.cache_dir)),
    ]);
    let vars = manifest_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .chain(runtime_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    for (key, value) in vars {
        env.insert(
            format!("{VAR_PREFIX}{}", env_var_suffix(key)),
            value.to_owned(),
        );
    }
    for (name, digest) in fingerprints {
        env.insert(
            format!("{FINGERPRINT_PREFIX}{}", env_var_suffix(name)),
            digest.clone(),
        );
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::ProjectPaths;
    use crate::project::Project;
    use std::path::PathBuf;

    fn workspace(branch: &str) -> Workspace {
        Workspace {
            root: PathBuf::from("/wt"),
            branch: branch.to_owned(),
            project: Project {
                name: "myproj".to_owned(),
                common_dir: PathBuf::from("/repo/.git"),
                paths: ProjectPaths::new("/cfg/cyamus/projects/myproj"),
                cache_dir: PathBuf::from("/cache/cyamus/projects/myproj"),
            },
        }
    }

    #[test]
    fn base_variables() {
        let env = build(&workspace("feature/My-Thing"), &BTreeMap::new(), &[], &[]);
        assert_eq!(env["CYAMUS_PROJECT"], "myproj");
        assert_eq!(env["CYAMUS_BRANCH"], "feature/My-Thing");
        assert_eq!(env["CYAMUS_BRANCH_SLUG"], "feature-my-thing");
        assert_eq!(env["CYAMUS_BRANCH_SNAKE"], "feature_my_thing");
        assert_eq!(env["CYAMUS_WORKSPACE"], "/wt");
        assert_eq!(env["CYAMUS_CONFIG_DIR"], "/cfg/cyamus/projects/myproj");
        assert_eq!(env["CYAMUS_ASSETS"], "/cfg/cyamus/projects/myproj/assets");
        assert_eq!(env["CYAMUS_BIN"], "/cfg/cyamus/projects/myproj/bin");
        assert_eq!(env["CYAMUS_CACHE_DIR"], "/cache/cyamus/projects/myproj");
        assert!(!env.contains_key("CYAMUS_EVENT"));
    }

    #[test]
    fn runtime_vars_override_manifest_vars() {
        let manifest = BTreeMap::from([
            ("env".to_owned(), "dev".to_owned()),
            ("node-version".to_owned(), "22".to_owned()),
        ]);
        let runtime = [("ENV".to_owned(), "staging".to_owned())];
        let env = build(&workspace("main"), &manifest, &runtime, &[]);
        assert_eq!(env["CYAMUS_VAR_ENV"], "staging");
        assert_eq!(env["CYAMUS_VAR_NODE_VERSION"], "22");
    }

    #[test]
    fn fingerprint_variables() {
        let fps = [("docker-deps".to_owned(), "abc123".to_owned())];
        let env = build(&workspace("main"), &BTreeMap::new(), &[], &fps);
        assert_eq!(env["CYAMUS_FINGERPRINT_DOCKER_DEPS"], "abc123");
    }
}
