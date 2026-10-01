//! Turns registered workspaces and running containers into a route table.
//!
//! Everything here is a pure function of its inputs, so the whole
//! docker-discovery contract is testable without Docker.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use cyamus_registry::{MAX_LABEL_LEN, Record};
use serde::{Deserialize, Serialize};

pub const LABEL_ENABLE: &str = "cyamus.enable";
pub const LABEL_SERVICE: &str = "cyamus.service";
pub const LABEL_PORT: &str = "cyamus.port";
pub const LABEL_PROJECT: &str = "cyamus.project";
pub const LABEL_WORKSPACE: &str = "cyamus.workspace";
pub const COMPOSE_WORKING_DIR: &str = "com.docker.compose.project.working_dir";
pub const COMPOSE_SERVICE: &str = "com.docker.compose.service";

/// The parts of a running container that routing looks at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    /// Without Docker's leading `/`.
    pub name: String,
    pub labels: HashMap<String, String>,
    pub ports: Vec<PortBinding>,
    /// Creation time (seconds since the epoch); earlier wins a hostname.
    pub created: i64,
}

/// One published port binding as Docker reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortBinding {
    pub container_port: u16,
    pub host_ip: Option<String>,
    pub host_port: Option<u16>,
    pub tcp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub target: SocketAddr,
    pub project: String,
    /// Branch label; `None` for the project's shared stack.
    pub workspace: Option<String>,
    pub service: String,
    pub container: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unrouted {
    pub container: String,
    pub project: Option<String>,
    pub workspace: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// Keyed by lowercase hostname.
    pub routes: BTreeMap<String, Route>,
    pub unrouted: Vec<Unrouted>,
}

impl Table {
    pub fn get(&self, host: &str) -> Option<&Route> {
        self.routes.get(host)
    }
}

/// A project's shared stack: its name and canonical config directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedStack {
    pub project: String,
    pub config_dir: PathBuf,
}

/// Who a container belongs to.
struct Owner {
    project: String,
    workspace: Option<String>,
}

/// Builds the route table. `canonicalize` resolves a compose working
/// directory to its canonical form (injected so tests need no filesystem).
pub fn resolve(
    workspaces: &[Record],
    shared: &[SharedStack],
    containers: &[Container],
    canonicalize: impl Fn(&Path) -> PathBuf,
) -> Table {
    let mut candidates: Vec<(&Container, Route)> = Vec::new();
    let mut unrouted = Vec::new();
    for container in containers {
        match route_for(workspaces, shared, container, &canonicalize) {
            Outcome::Ignore => {}
            Outcome::Unrouted(u) => unrouted.push(u),
            Outcome::Route(route) => candidates.push((container, route)),
        }
    }
    // Earliest container wins a contested hostname; the name breaks ties so
    // the outcome is stable.
    candidates.sort_by(|(a, _), (b, _)| (a.created, &a.name).cmp(&(b.created, &b.name)));
    let mut routes: BTreeMap<String, Route> = BTreeMap::new();
    for (_, route) in candidates {
        if let Some(winner) = routes.get(&route.host) {
            unrouted.push(Unrouted {
                container: route.container.clone(),
                project: Some(route.project.clone()),
                workspace: route.workspace.clone(),
                reason: format!(
                    "duplicate of {} for {} (the older container wins)",
                    winner.container, route.host
                ),
            });
        } else {
            routes.insert(route.host.clone(), route);
        }
    }
    unrouted.sort_by(|a, b| a.container.cmp(&b.container));
    Table { routes, unrouted }
}

enum Outcome {
    /// Not ours: no workspace and no cyamus labels. Not worth reporting.
    Ignore,
    Unrouted(Unrouted),
    Route(Route),
}

fn route_for(
    workspaces: &[Record],
    shared: &[SharedStack],
    c: &Container,
    canonicalize: &impl Fn(&Path) -> PathBuf,
) -> Outcome {
    let has_cyamus_labels = c.labels.keys().any(|k| k.starts_with("cyamus."));
    let unrouted = |owner: Option<&Owner>, reason: String| {
        Outcome::Unrouted(Unrouted {
            container: c.name.clone(),
            project: owner.map(|o| o.project.clone()),
            workspace: owner.and_then(|o| o.workspace.clone()),
            reason,
        })
    };

    let owner = match membership(workspaces, shared, c, canonicalize) {
        Ok(Some(owner)) => owner,
        Ok(None) if has_cyamus_labels => {
            return unrouted(None, "does not belong to any registered workspace".into());
        }
        Ok(None) => return Outcome::Ignore,
        Err(reason) => return unrouted(None, reason),
    };
    let ws = Some(&owner);

    match c.labels.get(LABEL_ENABLE).map(String::as_str) {
        None | Some("true") => {}
        Some("false") => return unrouted(ws, format!("opted out ({LABEL_ENABLE}=false)")),
        Some(other) => {
            return unrouted(
                ws,
                format!("invalid {LABEL_ENABLE}={other:?} (expected true or false)"),
            );
        }
    }

    let raw_service = c
        .labels
        .get(LABEL_SERVICE)
        .or_else(|| c.labels.get(COMPOSE_SERVICE))
        .unwrap_or(&c.name);
    let service = sanitize(raw_service);
    if service.is_empty() || service.len() > MAX_LABEL_LEN {
        return unrouted(
            ws,
            format!("service name {raw_service:?} is not a valid hostname label"),
        );
    }

    let target = match target(c) {
        Ok(t) => t,
        Err(reason) => return unrouted(ws, reason),
    };

    let host = match &owner.workspace {
        Some(label) => format!("{service}.{label}.{}.localhost", owner.project),
        None => format!("{service}.{}.localhost", owner.project),
    };
    Outcome::Route(Route {
        host,
        target,
        project: owner.project,
        workspace: owner.workspace,
        service,
        container: c.name.clone(),
    })
}

fn membership(
    workspaces: &[Record],
    shared: &[SharedStack],
    c: &Container,
    canonicalize: &impl Fn(&Path) -> PathBuf,
) -> Result<Option<Owner>, String> {
    let of = |w: &Record| Owner {
        project: w.project.clone(),
        workspace: Some(w.label.clone()),
    };
    let project = c.labels.get(LABEL_PROJECT);
    let label = c.labels.get(LABEL_WORKSPACE);
    match (project, label) {
        (Some(project), Some(label)) => {
            return workspaces
                .iter()
                .find(|w| &w.project == project && &w.label == label)
                .map(|w| Some(of(w)))
                .ok_or_else(|| {
                    format!("no registered workspace {label:?} in project {project:?}")
                });
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(format!(
                "{LABEL_PROJECT} and {LABEL_WORKSPACE} must be set together"
            ));
        }
        (None, None) => {}
    }
    let Some(dir) = c.labels.get(COMPOSE_WORKING_DIR) else {
        return Ok(None);
    };
    let dir = canonicalize(Path::new(dir));
    if let Some(stack) = shared.iter().find(|s| s.config_dir == dir) {
        return Ok(Some(Owner {
            project: stack.project.clone(),
            workspace: None,
        }));
    }
    Ok(workspaces
        .iter()
        .filter(|w| dir.starts_with(&w.path))
        .max_by_key(|w| w.path.components().count())
        .map(of))
}

fn target(c: &Container) -> Result<SocketAddr, String> {
    let published: Vec<&PortBinding> = c
        .ports
        .iter()
        .filter(|p| p.tcp && p.host_port.is_some())
        .collect();
    let mut container_ports: Vec<u16> = published.iter().map(|p| p.container_port).collect();
    container_ports.sort_unstable();
    container_ports.dedup();

    let wanted = match c.labels.get(LABEL_PORT) {
        Some(raw) => {
            let port: u16 =
                raw.trim().parse().ok().filter(|p| *p != 0).ok_or_else(|| {
                    format!("invalid {LABEL_PORT}={raw:?} (expected a port number)")
                })?;
            if !container_ports.contains(&port) {
                return Err(format!(
                    "container port {port} from {LABEL_PORT} is not published"
                ));
            }
            port
        }
        None => match container_ports.as_slice() {
            [] => return Err("no published TCP port".into()),
            [only] => *only,
            many => {
                let list: Vec<String> = many.iter().map(u16::to_string).collect();
                return Err(format!(
                    "publishes several ports ({}); set {LABEL_PORT} to pick one",
                    list.join(", ")
                ));
            }
        },
    };

    let bindings: Vec<&&PortBinding> = published
        .iter()
        .filter(|p| p.container_port == wanted)
        .collect();
    let ip_of = |b: &PortBinding| -> Option<IpAddr> {
        b.host_ip
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
    };
    // Prefer IPv4 loopback, then IPv6 loopback, then whatever IP is bound.
    let pick = |want: fn(&IpAddr) -> bool| {
        bindings
            .iter()
            .find(|b| ip_of(b).is_none_or(|ip| want(&ip)))
            .copied()
    };
    let (ip, binding) = if let Some(b) =
        pick(|ip| ip.is_ipv4() && (ip.is_unspecified() || ip.is_loopback()))
    {
        (IpAddr::V4(Ipv4Addr::LOCALHOST), b)
    } else if let Some(b) = pick(|ip| ip.is_ipv6() && (ip.is_unspecified() || ip.is_loopback())) {
        (IpAddr::V6(Ipv6Addr::LOCALHOST), b)
    } else {
        let b = bindings[0];
        let ip = ip_of(b).ok_or_else(|| format!("unusable host IP {:?}", b.host_ip))?;
        (ip, b)
    };
    let port = binding.host_port.expect("filtered to published bindings");
    Ok(SocketAddr::new(ip, port))
}

/// Lowercases and collapses every run outside `[a-z0-9]` into `-`.
pub fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending = false;
    for c in raw.chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending && !out.is_empty() {
                out.push('-');
            }
            pending = false;
            out.push(c);
        } else {
            pending = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(project: &str, label: &str, path: &str) -> Record {
        Record {
            project: project.into(),
            branch: label.into(),
            label: label.into(),
            path: path.into(),
        }
    }

    fn workspaces() -> Vec<Record> {
        vec![
            ws("myproj", "feat-x", "/w/feat"),
            ws("myproj", "main", "/w/main"),
            ws("other", "nested", "/w/feat/vendor/other"),
        ]
    }

    fn shared() -> Vec<SharedStack> {
        vec![SharedStack {
            project: "myproj".into(),
            config_dir: "/cfg/cyamus/projects/myproj".into(),
        }]
    }

    fn bind(container_port: u16, ip: &str, host_port: u16) -> PortBinding {
        PortBinding {
            container_port,
            host_ip: Some(ip.into()),
            host_port: Some(host_port),
            tcp: true,
        }
    }

    fn compose(name: &str, service: &str, dir: &str, ports: Vec<PortBinding>) -> Container {
        Container {
            id: format!("id-{name}"),
            name: name.into(),
            labels: HashMap::from([
                (COMPOSE_WORKING_DIR.into(), dir.into()),
                (COMPOSE_SERVICE.into(), service.into()),
            ]),
            ports,
            created: 100,
        }
    }

    fn with(mut c: Container, labels: &[(&str, &str)]) -> Container {
        for (k, v) in labels {
            c.labels.insert((*k).into(), (*v).into());
        }
        c
    }

    fn table(containers: &[Container]) -> Table {
        resolve(&workspaces(), &shared(), containers, Path::to_path_buf)
    }

    fn only_reason(t: &Table) -> &str {
        assert!(t.routes.is_empty(), "{t:?}");
        assert_eq!(t.unrouted.len(), 1, "{t:?}");
        &t.unrouted[0].reason
    }

    #[test]
    fn compose_container_in_worktree() {
        let t = table(&[compose(
            "feat-web-1",
            "web",
            "/w/feat",
            vec![bind(3000, "0.0.0.0", 32768)],
        )]);
        let route = t.get("web.feat-x.myproj.localhost").unwrap();
        assert_eq!(route.target, "127.0.0.1:32768".parse().unwrap());
        assert_eq!(route.container, "feat-web-1");
        assert!(t.unrouted.is_empty());
    }

    #[test]
    fn compose_from_subdirectory_uses_longest_match() {
        let t = table(&[
            compose("a", "web", "/w/feat/infra", vec![bind(3000, "0.0.0.0", 1)]),
            compose(
                "b",
                "api",
                "/w/feat/vendor/other/x",
                vec![bind(3000, "0.0.0.0", 2)],
            ),
        ]);
        assert!(t.get("web.feat-x.myproj.localhost").is_some());
        assert!(t.get("api.nested.other.localhost").is_some());
    }

    #[test]
    fn path_prefix_respects_components() {
        // /w/feature is not inside /w/feat.
        let t = table(&[compose(
            "a",
            "web",
            "/w/feature",
            vec![bind(3000, "0.0.0.0", 1)],
        )]);
        assert!(t.routes.is_empty() && t.unrouted.is_empty());
    }

    #[test]
    fn outside_any_worktree_is_ignored() {
        let t = table(&[compose(
            "a",
            "web",
            "/elsewhere",
            vec![bind(3000, "0.0.0.0", 1)],
        )]);
        assert_eq!(t, Table::default());
    }

    #[test]
    fn explicit_labels_attach_non_compose_container() {
        let c = Container {
            name: "adhoc".into(),
            labels: HashMap::from([
                (LABEL_PROJECT.into(), "myproj".into()),
                (LABEL_WORKSPACE.into(), "main".into()),
            ]),
            ports: vec![bind(8080, "127.0.0.1", 40000)],
            ..Container::default()
        };
        let t = table(&[c]);
        let route = t.get("adhoc.main.myproj.localhost").unwrap();
        assert_eq!(route.target, "127.0.0.1:40000".parse().unwrap());
    }

    #[test]
    fn explicit_labels_take_precedence_over_working_dir() {
        let c = with(
            compose("a", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]),
            &[(LABEL_PROJECT, "myproj"), (LABEL_WORKSPACE, "main")],
        );
        assert!(table(&[c]).get("web.main.myproj.localhost").is_some());
    }

    #[test]
    fn half_explicit_labels_are_reported() {
        let c = with(
            compose("a", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]),
            &[(LABEL_PROJECT, "myproj")],
        );
        assert!(only_reason(&table(&[c])).contains("must be set together"));
    }

    #[test]
    fn unknown_explicit_workspace_is_reported() {
        let c = Container {
            name: "adhoc".into(),
            labels: HashMap::from([
                (LABEL_PROJECT.into(), "myproj".into()),
                (LABEL_WORKSPACE.into(), "gone".into()),
            ]),
            ..Container::default()
        };
        assert!(only_reason(&table(&[c])).contains("no registered workspace"));
    }

    #[test]
    fn opt_out() {
        let c = with(
            compose("db", "db", "/w/feat", vec![bind(5432, "0.0.0.0", 1)]),
            &[(LABEL_ENABLE, "false")],
        );
        let t = table(&[c]);
        assert!(only_reason(&t).contains("opted out"));
        assert_eq!(t.unrouted[0].workspace.as_deref(), Some("feat-x"));
    }

    #[test]
    fn invalid_enable_value() {
        let c = with(
            compose("db", "db", "/w/feat", vec![bind(5432, "0.0.0.0", 1)]),
            &[(LABEL_ENABLE, "no")],
        );
        assert!(only_reason(&table(&[c])).contains("invalid cyamus.enable"));
    }

    #[test]
    fn service_label_renames() {
        let c = with(
            compose("api", "api", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]),
            &[(LABEL_SERVICE, "Back_End")],
        );
        assert!(
            table(&[c])
                .get("back-end.feat-x.myproj.localhost")
                .is_some()
        );
    }

    #[test]
    fn container_name_is_last_resort() {
        let c = Container {
            name: "My_Container".into(),
            labels: HashMap::from([(COMPOSE_WORKING_DIR.into(), "/w/feat".into())]),
            ports: vec![bind(80, "0.0.0.0", 1)],
            ..Container::default()
        };
        assert!(
            table(&[c])
                .get("my-container.feat-x.myproj.localhost")
                .is_some()
        );
    }

    #[test]
    fn unusable_service_name() {
        let c = with(
            compose("a", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]),
            &[(LABEL_SERVICE, "___")],
        );
        assert!(only_reason(&table(&[c])).contains("not a valid hostname label"));
        let long = "s".repeat(64);
        let c = with(
            compose("a", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]),
            &[(LABEL_SERVICE, long.as_str())],
        );
        assert!(only_reason(&table(&[c])).contains("not a valid hostname label"));
    }

    #[test]
    fn ipv4_and_ipv6_bindings_count_once() {
        let c = compose(
            "a",
            "web",
            "/w/feat",
            vec![bind(3000, "0.0.0.0", 32768), bind(3000, "::", 32768)],
        );
        let t = table(&[c]);
        assert_eq!(
            t.get("web.feat-x.myproj.localhost").unwrap().target,
            "127.0.0.1:32768".parse().unwrap()
        );
    }

    #[test]
    fn ipv6_only_binding_targets_ipv6_loopback() {
        let c = compose("a", "web", "/w/feat", vec![bind(3000, "::", 5000)]);
        assert_eq!(
            table(&[c])
                .get("web.feat-x.myproj.localhost")
                .unwrap()
                .target,
            "[::1]:5000".parse().unwrap()
        );
    }

    #[test]
    fn specific_host_ip_is_used() {
        let c = compose("a", "web", "/w/feat", vec![bind(3000, "192.168.1.5", 5000)]);
        assert_eq!(
            table(&[c])
                .get("web.feat-x.myproj.localhost")
                .unwrap()
                .target,
            "192.168.1.5:5000".parse().unwrap()
        );
    }

    #[test]
    fn several_ports_need_a_label() {
        let ports = vec![bind(3000, "0.0.0.0", 1), bind(9229, "0.0.0.0", 2)];
        let reason =
            only_reason(&table(&[compose("a", "web", "/w/feat", ports.clone())])).to_owned();
        assert!(reason.contains("3000, 9229"), "{reason}");
        let c = with(
            compose("a", "web", "/w/feat", ports),
            &[(LABEL_PORT, "3000")],
        );
        assert_eq!(
            table(&[c])
                .get("web.feat-x.myproj.localhost")
                .unwrap()
                .target,
            "127.0.0.1:1".parse().unwrap()
        );
    }

    #[test]
    fn port_label_errors() {
        let base = compose("a", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]);
        let c = with(base.clone(), &[(LABEL_PORT, "8080")]);
        assert!(only_reason(&table(&[c])).contains("8080 from cyamus.port is not published"));
        let c = with(base, &[(LABEL_PORT, "http")]);
        assert!(only_reason(&table(&[c])).contains("invalid cyamus.port"));
    }

    #[test]
    fn no_published_port() {
        let mut c = compose("a", "web", "/w/feat", vec![]);
        c.ports.push(PortBinding {
            container_port: 3000,
            host_ip: None,
            host_port: None,
            tcp: true,
        });
        c.ports.push(PortBinding {
            container_port: 53,
            host_ip: Some("0.0.0.0".into()),
            host_port: Some(53),
            tcp: false,
        });
        assert!(only_reason(&table(&[c])).contains("no published TCP port"));
    }

    #[test]
    fn duplicates_keep_the_oldest() {
        let mut first = compose("web-2", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 2)]);
        first.created = 10;
        let mut second = compose("web-1", "web", "/w/feat", vec![bind(3000, "0.0.0.0", 1)]);
        second.created = 20;
        let t = table(&[second, first]);
        assert_eq!(
            t.get("web.feat-x.myproj.localhost").unwrap().container,
            "web-2"
        );
        assert_eq!(t.unrouted.len(), 1);
        assert_eq!(t.unrouted[0].container, "web-1");
        assert!(t.unrouted[0].reason.contains("duplicate of web-2"));
    }

    #[test]
    fn working_dir_is_canonicalized() {
        let c = compose("a", "web", "/tmp/w/feat", vec![bind(3000, "0.0.0.0", 1)]);
        let t = resolve(&workspaces(), &shared(), &[c], |p| {
            PathBuf::from(p.to_string_lossy().replacen("/tmp", "", 1))
        });
        assert!(t.get("web.feat-x.myproj.localhost").is_some());
    }

    #[test]
    fn shared_stack_routes_without_branch() {
        let c = compose(
            "myproj-mailpit-1",
            "mailpit",
            "/cfg/cyamus/projects/myproj",
            vec![bind(8025, "0.0.0.0", 49200)],
        );
        let t = table(&[c]);
        let route = t.get("mailpit.myproj.localhost").unwrap();
        assert_eq!(route.workspace, None);
        assert_eq!(route.target, "127.0.0.1:49200".parse().unwrap());
    }

    #[test]
    fn shared_stack_opt_out_and_other_dirs() {
        let db = with(
            compose(
                "myproj-db-1",
                "db",
                "/cfg/cyamus/projects/myproj",
                vec![bind(5432, "0.0.0.0", 1)],
            ),
            &[(LABEL_ENABLE, "false")],
        );
        let t = table(&[db]);
        assert!(only_reason(&t).contains("opted out"));
        assert_eq!(t.unrouted[0].project.as_deref(), Some("myproj"));
        assert_eq!(t.unrouted[0].workspace, None);
        // A subdirectory of a config dir is not the shared stack.
        let sub = compose(
            "x",
            "web",
            "/cfg/cyamus/projects/myproj/assets",
            vec![bind(80, "0.0.0.0", 1)],
        );
        assert_eq!(table(&[sub]), Table::default());
    }

    #[test]
    fn explicit_labels_beat_shared_dir() {
        let c = with(
            compose(
                "a",
                "web",
                "/cfg/cyamus/projects/myproj",
                vec![bind(80, "0.0.0.0", 1)],
            ),
            &[(LABEL_PROJECT, "myproj"), (LABEL_WORKSPACE, "main")],
        );
        assert!(table(&[c]).get("web.main.myproj.localhost").is_some());
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize("Web_App--1"), "web-app-1");
        assert_eq!(sanitize("--x--"), "x");
        assert_eq!(sanitize("_"), "");
    }
}
