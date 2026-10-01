//! The `CYAMUS_*` environment exposed to hooks.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::naming::{env_var_suffix, slugify};
use crate::workspace::Workspace;

pub const VAR_PREFIX: &str = "CYAMUS_VAR_";
pub const FINGERPRINT_PREFIX: &str = "CYAMUS_FINGERPRINT_";
pub const COMPOSE_PROJECT_NAME: &str = "COMPOSE_PROJECT_NAME";

/// Hostname information for a workspace, when it has a usable branch label.
#[derive(Debug, Clone, Copy)]
pub struct Routing<'a> {
    pub label: Option<&'a str>,
    pub proxy_port: u16,
    pub url_suffix: &'a str,
}

/// Builds the variables exposed to hooks (everything except `CYAMUS_EVENT`,
/// which the hook runner sets per event).
///
/// Runtime variables override manifest variables with the same normalized
/// name. Hooks inherit the parent process environment on top of these;
/// `COMPOSE_PROJECT_NAME` is only included when the caller doesn't set it.
pub fn build(
    workspace: &Workspace,
    manifest_vars: &BTreeMap<String, String>,
    runtime_vars: &[(String, String)],
    fingerprints: &[(String, String)],
    routing: Routing<'_>,
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
        (
            "CYAMUS_PROXY_PORT".to_owned(),
            routing.proxy_port.to_string(),
        ),
        (
            "CYAMUS_URL_SUFFIX".to_owned(),
            routing.url_suffix.to_owned(),
        ),
    ]);
    if let Some(label) = routing.label {
        env.insert(
            "CYAMUS_DOMAIN".to_owned(),
            format!("{label}.{}.localhost", project.name),
        );
        if std::env::var_os(COMPOSE_PROJECT_NAME).is_none() {
            env.insert(
                COMPOSE_PROJECT_NAME.to_owned(),
                format!("{}-{label}", project.name),
            );
        }
    }
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

/// `PATH` with `dir` moved to the front (later duplicates dropped), or
/// `None` when it already comes first. Hooks get this so that `cyamus`
/// inside a hook is the binary running setup, even when the caller's PATH
/// lacks it (Orca started from the Dock).
pub fn path_with_first(dir: &Path, path: Option<&OsStr>) -> Option<OsString> {
    let rest: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    if rest.first().is_some_and(|first| first == dir) {
        return None;
    }
    let entries = std::iter::once(dir.to_owned()).chain(rest.into_iter().filter(|p| p != dir));
    std::env::join_paths(entries).ok()
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

    const ROUTING: Routing<'static> = Routing {
        label: Some("feature-my-thing"),
        proxy_port: 1355,
        url_suffix: ":1355",
    };

    #[test]
    fn base_variables() {
        let env = build(
            &workspace("feature/My-Thing"),
            &BTreeMap::new(),
            &[],
            &[],
            ROUTING,
        );
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
        assert_eq!(env["CYAMUS_DOMAIN"], "feature-my-thing.myproj.localhost");
        assert_eq!(env["CYAMUS_PROXY_PORT"], "1355");
        assert_eq!(env["CYAMUS_URL_SUFFIX"], ":1355");
    }

    #[test]
    fn path_puts_dir_first() {
        let dir = Path::new("/opt/cyamus/bin");
        let p = |s: &str| Some(OsString::from(s));
        assert_eq!(
            path_with_first(dir, Some(OsStr::new("/usr/bin:/bin"))),
            p("/opt/cyamus/bin:/usr/bin:/bin")
        );
        assert_eq!(
            path_with_first(dir, Some(OsStr::new("/opt/cyamus/bin:/usr/bin"))),
            None
        );
        assert_eq!(
            path_with_first(dir, Some(OsStr::new("/usr/bin:/opt/cyamus/bin"))),
            p("/opt/cyamus/bin:/usr/bin")
        );
        assert_eq!(path_with_first(dir, None), p("/opt/cyamus/bin"));
    }

    #[test]
    fn no_domain_without_label() {
        let routing = Routing {
            label: None,
            proxy_port: 4000,
            url_suffix: "",
        };
        let env = build(&workspace("_"), &BTreeMap::new(), &[], &[], routing);
        assert!(!env.contains_key("CYAMUS_DOMAIN"));
        assert!(!env.contains_key(COMPOSE_PROJECT_NAME));
        assert_eq!(env["CYAMUS_PROXY_PORT"], "4000");
    }

    #[test]
    fn runtime_vars_override_manifest_vars() {
        let manifest = BTreeMap::from([
            ("env".to_owned(), "dev".to_owned()),
            ("node-version".to_owned(), "22".to_owned()),
        ]);
        let runtime = [("ENV".to_owned(), "staging".to_owned())];
        let env = build(&workspace("main"), &manifest, &runtime, &[], ROUTING);
        assert_eq!(env["CYAMUS_VAR_ENV"], "staging");
        assert_eq!(env["CYAMUS_VAR_NODE_VERSION"], "22");
    }

    #[test]
    fn fingerprint_variables() {
        let fps = [("docker-deps".to_owned(), "abc123".to_owned())];
        let env = build(&workspace("main"), &BTreeMap::new(), &[], &fps, ROUTING);
        assert_eq!(env["CYAMUS_FINGERPRINT_DOCKER_DEPS"], "abc123");
    }
}
