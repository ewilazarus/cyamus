//! Shared daemon state: the registry and Docker snapshots, and the route table
//! derived from them.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use cyamus_registry::{ChangeKey, Record, Registry};

use crate::docker::{Endpoint, Snapshot};
use crate::routes::{self, SharedStack, Table};
use crate::status::{DockerStatus, Status};

pub struct State {
    registry: Registry,
    /// `$XDG_CONFIG_HOME/cyamus/projects`: each directory is a project whose
    /// shared stack (`cyamus compose-shared`) runs from there.
    projects_dir: std::path::PathBuf,
    endpoint: Endpoint,
    port: u16,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    registry_key: Option<ChangeKey>,
    records: Vec<Record>,
    shared: Vec<SharedStack>,
    docker: Snapshot,
    /// Set when the Docker snapshot changed since the table was built.
    docker_dirty: bool,
    table: Arc<Table>,
}

impl State {
    pub fn new(
        registry: Registry,
        projects_dir: impl Into<std::path::PathBuf>,
        endpoint: Endpoint,
        port: u16,
    ) -> Self {
        Self {
            registry,
            projects_dir: projects_dir.into(),
            endpoint,
            port,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The current route table. Re-reads the registry when it changed on
    /// disk, so a registration is visible to the very next request.
    pub fn table(&self) -> Arc<Table> {
        let key = self.registry.change_key();
        let shared = shared_stacks(&self.projects_dir);
        let mut inner = self.lock();
        let registry_changed = inner.registry_key.as_ref() != Some(&key);
        if registry_changed {
            inner.records = self.registry.read_all().unwrap_or_default();
            inner.registry_key = Some(key);
        }
        let shared_changed = inner.shared != shared;
        if shared_changed {
            inner.shared = shared;
        }
        if registry_changed || shared_changed || inner.docker_dirty {
            let table = routes::resolve(
                &inner.records,
                &inner.shared,
                &inner.docker.containers,
                canonicalize,
            );
            inner.table = Arc::new(table);
            inner.docker_dirty = false;
        }
        Arc::clone(&inner.table)
    }

    pub fn set_docker(&self, snapshot: Snapshot) {
        let mut inner = self.lock();
        inner.docker = snapshot;
        inner.docker_dirty = true;
    }

    pub fn docker_reachable(&self) -> bool {
        self.lock().docker.reachable
    }

    pub fn status(&self) -> Status {
        let table = self.table();
        let inner = self.lock();
        Status {
            pid: std::process::id(),
            version: crate::VERSION.to_owned(),
            port: self.port,
            docker: DockerStatus {
                reachable: inner.docker.reachable,
                endpoint: match &self.endpoint.socket {
                    Ok(p) => p.display().to_string(),
                    Err(_) => String::new(),
                },
                source: self.endpoint.source.clone(),
                error: inner.docker.error.clone(),
            },
            routes: table.routes.values().cloned().collect(),
            unrouted: table.unrouted.clone(),
        }
    }
}

/// One shared stack per directory under `projects_dir`, canonicalized.
fn shared_stacks(projects_dir: &Path) -> Vec<SharedStack> {
    let Ok(entries) = std::fs::read_dir(projects_dir) else {
        return Vec::new();
    };
    let mut stacks: Vec<SharedStack> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| SharedStack {
            project: e.file_name().to_string_lossy().into_owned(),
            config_dir: canonicalize(&e.path()),
        })
        .collect();
    stacks.sort_by(|a, b| a.project.cmp(&b.project));
    stacks
}

fn canonicalize(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::routes::{COMPOSE_WORKING_DIR, Container, PortBinding};

    #[test]
    fn registry_changes_are_picked_up_per_request() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let wt = root.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let registry = Registry::new(root.join("ws"), root.join("lock"));
        let endpoint = Endpoint {
            socket: Err("none".into()),
            source: "test".into(),
        };
        let state = State::new(registry.clone(), root.join("projects"), endpoint, 1355);
        state.set_docker(Snapshot {
            reachable: true,
            error: None,
            containers: vec![Container {
                name: "web-1".into(),
                labels: HashMap::from([(COMPOSE_WORKING_DIR.into(), wt.display().to_string())]),
                ports: vec![PortBinding {
                    container_port: 3000,
                    host_ip: Some("0.0.0.0".into()),
                    host_port: Some(32768),
                    tcp: true,
                }],
                ..Container::default()
            }],
        });
        assert!(state.table().routes.is_empty());

        registry
            .register(&Record {
                project: "myproj".into(),
                branch: "feat".into(),
                label: "feat".into(),
                path: wt.clone(),
            })
            .unwrap();
        assert!(state.table().get("web-1.feat.myproj.localhost").is_some());

        registry.unregister("myproj", "feat", &wt).unwrap();
        assert!(state.table().routes.is_empty());
    }

    #[test]
    fn new_project_dirs_are_picked_up_without_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let projects = root.join("projects");
        let registry = Registry::new(root.join("ws"), root.join("lock"));
        let endpoint = Endpoint {
            socket: Err("none".into()),
            source: "test".into(),
        };
        let state = State::new(registry, &projects, endpoint, 1355);
        let config = projects.join("myproj");
        state.set_docker(Snapshot {
            reachable: true,
            error: None,
            containers: vec![Container {
                name: "myproj-mailpit-1".into(),
                labels: HashMap::from([
                    (COMPOSE_WORKING_DIR.into(), config.display().to_string()),
                    ("com.docker.compose.service".into(), "mailpit".into()),
                ]),
                ports: vec![PortBinding {
                    container_port: 8025,
                    host_ip: Some("0.0.0.0".into()),
                    host_port: Some(49200),
                    tcp: true,
                }],
                ..Container::default()
            }],
        });
        assert!(state.table().routes.is_empty());
        std::fs::create_dir_all(&config).unwrap();
        let table = state.table();
        let route = table.get("mailpit.myproj.localhost").unwrap();
        assert_eq!(route.workspace, None);
    }
}
