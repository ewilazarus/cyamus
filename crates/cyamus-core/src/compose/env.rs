//! The environments the two stacks run compose with.
//!
//! Worktree stack: everything hooks get, added only where missing, so a hook's
//! runtime `--var` overrides win and a terminal gets the manifest values.
//! Shared stack: project-level values set explicitly and worktree variables
//! stripped, so it is configured the same whichever worktree started it.

use std::collections::BTreeMap;

use crate::daemon::DaemonControl;
use crate::env::{self, FINGERPRINT_PREFIX, Routing, VAR_PREFIX};
use crate::fingerprint;
use crate::naming::env_var_suffix;

use super::Context;

/// Variables to add for the worktree stack: the hook environment minus
/// anything already present in `inherited`.
pub fn worktree(
    ctx: &Context,
    daemon: &dyn DaemonControl,
    inherited: &dyn Fn(&str) -> bool,
) -> BTreeMap<String, String> {
    let (vars, fingerprints) = match &ctx.manifest {
        Some(m) => (
            m.vars.clone(),
            fingerprint::compute(&ctx.workspace.root, &m.fingerprints),
        ),
        None => Default::default(),
    };
    let url_suffix = daemon.url_suffix();
    let routing = Routing {
        label: ctx.label.as_deref(),
        proxy_port: daemon.port(),
        url_suffix: &url_suffix,
    };
    env::build(&ctx.workspace, &vars, &[], &fingerprints, routing)
        .into_iter()
        .filter(|(k, _)| !inherited(k))
        .collect()
}

/// Worktree-specific variables that never reach the shared stack.
const WORKTREE_ONLY: &[&str] = &[
    "CYAMUS_BRANCH",
    "CYAMUS_BRANCH_SLUG",
    "CYAMUS_BRANCH_SNAKE",
    "CYAMUS_WORKSPACE",
    "CYAMUS_EVENT",
    "COMPOSE_PROJECT_NAME",
    "COMPOSE_FILE",
];

/// `(set, remove)` for the shared stack. `inherited` lists the caller's
/// variable names, so inherited `CYAMUS_VAR_*`/`CYAMUS_FINGERPRINT_*` can be
/// removed.
pub fn shared(
    ctx: &Context,
    daemon: &dyn DaemonControl,
    inherited: impl Iterator<Item = String>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let project = &ctx.workspace.project;
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let mut set = BTreeMap::from([
        ("CYAMUS_PROJECT".to_owned(), project.name.clone()),
        ("CYAMUS_CONFIG_DIR".to_owned(), path(&project.paths.root)),
        ("CYAMUS_ASSETS".to_owned(), path(&project.paths.assets())),
        ("CYAMUS_BIN".to_owned(), path(&project.paths.bin())),
        ("CYAMUS_CACHE_DIR".to_owned(), path(&project.cache_dir)),
        (
            "CYAMUS_DOMAIN".to_owned(),
            format!("{}.localhost", project.name),
        ),
        ("CYAMUS_PROXY_PORT".to_owned(), daemon.port().to_string()),
        ("CYAMUS_URL_SUFFIX".to_owned(), daemon.url_suffix()),
    ]);
    if let Some(m) = &ctx.manifest {
        for (k, v) in &m.vars {
            set.insert(format!("{VAR_PREFIX}{}", env_var_suffix(k)), v.clone());
        }
    }
    let mut remove: Vec<String> = WORKTREE_ONLY.iter().map(|s| (*s).to_owned()).collect();
    remove.extend(inherited.filter(|k| {
        (k.starts_with(VAR_PREFIX) || k.starts_with(FINGERPRINT_PREFIX)) && !set.contains_key(k)
    }));
    (set, remove)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::NoDaemon;
    use crate::manifest::Manifest;
    use crate::paths::ProjectPaths;
    use crate::project::Project;
    use crate::workspace::Workspace;
    use std::path::{Path, PathBuf};

    fn ctx(with_manifest: bool) -> Context {
        let manifest = with_manifest.then(|| {
            Manifest::parse("[vars]\nnode-version = \"22\"\n", Path::new("m.toml")).unwrap()
        });
        Context {
            workspace: Workspace {
                root: PathBuf::from("/nonexistent/wt"),
                branch: "feature/X".into(),
                project: Project {
                    name: "myproj".into(),
                    common_dir: PathBuf::from("/repo/.git"),
                    paths: ProjectPaths::new("/cfg/cyamus/projects/myproj"),
                    cache_dir: PathBuf::from("/cache/myproj"),
                },
            },
            manifest,
            label: Some("feature-x".into()),
        }
    }

    #[test]
    fn worktree_fills_in_missing_only() {
        let daemon = NoDaemon { port: 1355 };
        let all = worktree(&ctx(true), &daemon, &|_| false);
        assert_eq!(all["CYAMUS_BRANCH"], "feature/X");
        assert_eq!(all["CYAMUS_DOMAIN"], "feature-x.myproj.localhost");
        assert_eq!(all["CYAMUS_URL_SUFFIX"], ":1355");
        assert_eq!(all["CYAMUS_VAR_NODE_VERSION"], "22");
        assert!(!all.contains_key("CYAMUS_EVENT"));
        // A hook's runtime override is inherited and must win.
        let some = worktree(&ctx(true), &daemon, &|k| k == "CYAMUS_VAR_NODE_VERSION");
        assert!(!some.contains_key("CYAMUS_VAR_NODE_VERSION"));
        assert_eq!(some["CYAMUS_PROJECT"], "myproj");
    }

    #[test]
    fn shared_is_project_level_and_deterministic() {
        let daemon = NoDaemon { port: 1355 };
        let inherited = [
            "CYAMUS_VAR_NODE_VERSION",
            "CYAMUS_VAR_FROM_HOOK",
            "CYAMUS_FINGERPRINT_DEPS",
            "PATH",
        ]
        .map(str::to_owned);
        let (set, remove) = shared(&ctx(true), &daemon, inherited.into_iter());
        assert_eq!(set["CYAMUS_DOMAIN"], "myproj.localhost");
        assert_eq!(
            set["CYAMUS_VAR_NODE_VERSION"], "22",
            "manifest value replaces the inherited one"
        );
        assert!(!set.contains_key("CYAMUS_BRANCH") && !set.contains_key("CYAMUS_WORKSPACE"));
        for gone in [
            "CYAMUS_BRANCH",
            "CYAMUS_WORKSPACE",
            "COMPOSE_PROJECT_NAME",
            "CYAMUS_VAR_FROM_HOOK",
            "CYAMUS_FINGERPRINT_DEPS",
        ] {
            assert!(remove.contains(&gone.to_owned()), "{gone} not removed");
        }
        assert!(!remove.contains(&"PATH".to_owned()));
        assert!(!remove.contains(&"CYAMUS_VAR_NODE_VERSION".to_owned()));
    }
}
