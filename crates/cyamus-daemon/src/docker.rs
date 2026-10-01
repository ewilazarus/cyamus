//! Docker discovery: where Docker listens, and a task that keeps a snapshot of
//! running containers up to date from Docker's event stream.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bollard::models::{ContainerSummary, PortSummaryTypeEnum};
use bollard::query_parameters::{EventsOptions, ListContainersOptions};
use bollard::{API_DEFAULT_VERSION, Docker};
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::routes::{Container, PortBinding};
use crate::state::State;

const DEFAULT_SOCKET: &str = "/var/run/docker.sock";
const DEBOUNCE: Duration = Duration::from_millis(200);
const MAX_BACKOFF: Duration = Duration::from_secs(5);
/// bollard applies this to whole requests, event stream included; the stream
/// simply ends and is re-established (with a fresh listing) when it expires.
const CLIENT_TIMEOUT_SECS: u64 = 600;

/// Where to reach Docker, and how that was decided (for status output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub socket: Result<PathBuf, String>,
    pub source: String,
}

/// Resolves the Docker endpoint: `DOCKER_HOST`, then the current Docker CLI
/// context (`DOCKER_CONTEXT` or `currentContext` in the CLI config), then
/// `/var/run/docker.sock`.
pub fn resolve_endpoint(lookup: impl Fn(&str) -> Option<OsString>) -> Endpoint {
    if let Some(host) = lookup("DOCKER_HOST").filter(|h| !h.is_empty()) {
        let host = host.to_string_lossy().into_owned();
        return Endpoint {
            socket: unix_socket(&host),
            source: "DOCKER_HOST".into(),
        };
    }
    let config_dir = lookup("DOCKER_CONFIG")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| lookup("HOME").map(|h| PathBuf::from(h).join(".docker")));
    let context = lookup("DOCKER_CONTEXT")
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string_lossy().into_owned())
        .or_else(|| config_dir.as_deref().and_then(current_context));
    if let (Some(context), Some(dir)) = (context, config_dir)
        && context != "default"
    {
        let socket = context_host(&dir, &context)
            .ok_or_else(|| format!("Docker context {context:?} has no endpoint"))
            .and_then(|host| unix_socket(&host));
        return Endpoint {
            socket,
            source: format!("Docker context {context:?}"),
        };
    }
    Endpoint {
        socket: Ok(PathBuf::from(DEFAULT_SOCKET)),
        source: "default socket".into(),
    }
}

fn unix_socket(host: &str) -> Result<PathBuf, String> {
    host.strip_prefix("unix://")
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!("unsupported Docker endpoint {host:?}; only unix:// sockets are supported")
        })
}

fn current_context(config_dir: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Config {
        #[serde(rename = "currentContext")]
        current_context: Option<String>,
    }
    let text = std::fs::read_to_string(config_dir.join("config.json")).ok()?;
    serde_json::from_str::<Config>(&text)
        .ok()?
        .current_context
        .filter(|c| !c.is_empty())
}

fn context_host(config_dir: &Path, context: &str) -> Option<String> {
    let digest = Sha256::digest(context.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let meta = config_dir.join("contexts/meta").join(hex).join("meta.json");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(meta).ok()?).ok()?;
    value
        .pointer("/Endpoints/docker/Host")?
        .as_str()
        .map(str::to_owned)
}

/// Docker's view as the daemon last saw it.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub reachable: bool,
    pub error: Option<String>,
    pub containers: Vec<Container>,
}

/// Keeps `state`'s Docker snapshot current until the process exits.
pub async fn watch(endpoint: Endpoint, state: Arc<State>) {
    let socket = match endpoint.socket.clone() {
        Ok(socket) => socket,
        Err(error) => {
            state.set_docker(Snapshot {
                error: Some(error),
                ..Snapshot::default()
            });
            return;
        }
    };
    let mut backoff = Duration::from_millis(250);
    loop {
        match session(&socket, &state).await {
            Ok(()) => backoff = Duration::from_millis(250),
            Err(error) => {
                if state.docker_reachable() {
                    crate::log(&format!("docker: {error}"));
                }
                state.set_docker(Snapshot {
                    error: Some(error),
                    ..Snapshot::default()
                });
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
        tokio::time::sleep(backoff).await;
    }
}

/// Connects, lists, and re-lists on every relevant event until the event
/// stream ends (Ok) or something fails (Err).
async fn session(socket: &Path, state: &State) -> Result<(), String> {
    let path = socket.to_string_lossy();
    let docker = Docker::connect_with_unix(&path, CLIENT_TIMEOUT_SECS, API_DEFAULT_VERSION)
        .map_err(|e| format!("cannot connect to {path}: {e}"))?;
    let filters = HashMap::from([
        ("type".to_owned(), vec!["container".to_owned()]),
        (
            "event".to_owned(),
            ["start", "die", "destroy", "rename", "pause", "unpause"]
                .map(str::to_owned)
                .to_vec(),
        ),
    ]);
    let mut events = docker.events(Some(EventsOptions {
        filters: Some(filters),
        ..EventsOptions::default()
    }));
    publish(&docker, state).await?;
    if !state.docker_reachable() {
        crate::log(&format!("docker: connected to {path}"));
    }
    while let Some(event) = events.next().await {
        event.map_err(|e| format!("event stream failed: {e}"))?;
        // Coalesce bursts (compose starting a whole stack) into one listing.
        let deadline = tokio::time::sleep(DEBOUNCE);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => break,
                next = events.next() => match next {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(format!("event stream failed: {e}")),
                    None => break,
                },
            }
        }
        publish(&docker, state).await?;
    }
    publish(&docker, state).await
}

async fn publish(docker: &Docker, state: &State) -> Result<(), String> {
    let summaries = docker
        .list_containers(Some(ListContainersOptions::default()))
        .await
        .map_err(|e| format!("cannot list containers: {e}"))?;
    state.set_docker(Snapshot {
        reachable: true,
        error: None,
        containers: summaries.into_iter().map(container).collect(),
    });
    Ok(())
}

fn container(s: ContainerSummary) -> Container {
    let name = s
        .names
        .and_then(|n| n.into_iter().next())
        .map(|n| n.trim_start_matches('/').to_owned())
        .unwrap_or_default();
    let ports = s
        .ports
        .unwrap_or_default()
        .into_iter()
        .map(|p| PortBinding {
            container_port: p.private_port,
            host_ip: p.ip,
            host_port: p.public_port,
            tcp: matches!(p.typ, Some(PortSummaryTypeEnum::TCP) | None),
        })
        .collect();
    Container {
        id: s.id.unwrap_or_default(),
        name,
        labels: s.labels.unwrap_or_default(),
        ports,
        created: s.created.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    fn resolve(vars: &[(&str, &str)]) -> Endpoint {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        resolve_endpoint(|k| map.get(k).cloned())
    }

    fn write_context(dir: &Path, name: &str, host: &str) {
        let hex: String = Sha256::digest(name.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let meta = dir.join("contexts/meta").join(hex);
        fs::create_dir_all(&meta).unwrap();
        fs::write(
            meta.join("meta.json"),
            format!(r#"{{"Name":"{name}","Endpoints":{{"docker":{{"Host":"{host}"}}}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn docker_host_wins() {
        let e = resolve(&[("DOCKER_HOST", "unix:///run/x.sock"), ("HOME", "/nope")]);
        assert_eq!(e.socket, Ok(PathBuf::from("/run/x.sock")));
        assert!(
            resolve(&[("DOCKER_HOST", "tcp://1.2.3.4:2375")])
                .socket
                .is_err()
        );
    }

    #[test]
    fn current_context_from_config() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".docker");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), r#"{"currentContext":"orbstack"}"#).unwrap();
        write_context(
            &dir,
            "orbstack",
            "unix:///Users/me/.orbstack/run/docker.sock",
        );
        let home = tmp.path().to_str().unwrap();
        let e = resolve(&[("HOME", home)]);
        assert_eq!(
            e.socket,
            Ok(PathBuf::from("/Users/me/.orbstack/run/docker.sock"))
        );
        assert!(e.source.contains("orbstack"));
    }

    #[test]
    fn docker_context_and_config_env() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cfg");
        write_context(&dir, "colima", "unix:///c.sock");
        let e = resolve(&[
            ("HOME", "/nope"),
            ("DOCKER_CONFIG", dir.to_str().unwrap()),
            ("DOCKER_CONTEXT", "colima"),
        ]);
        assert_eq!(e.socket, Ok(PathBuf::from("/c.sock")));
        let missing = resolve(&[
            ("DOCKER_CONFIG", dir.to_str().unwrap()),
            ("DOCKER_CONTEXT", "gone"),
        ]);
        assert!(missing.socket.is_err());
    }

    #[test]
    fn default_socket() {
        assert_eq!(
            resolve(&[("HOME", "/nope")]).socket,
            Ok(PathBuf::from(DEFAULT_SOCKET))
        );
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".docker");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), r#"{"currentContext":"default"}"#).unwrap();
        let e = resolve(&[("HOME", tmp.path().to_str().unwrap())]);
        assert_eq!(e.socket, Ok(PathBuf::from(DEFAULT_SOCKET)));
    }

    #[test]
    fn summary_conversion() {
        let s = ContainerSummary {
            id: Some("abc".into()),
            names: Some(vec!["/web-1".into()]),
            created: Some(42),
            labels: Some(HashMap::from([("a".into(), "b".into())])),
            ports: Some(vec![bollard::models::PortSummary {
                ip: Some("0.0.0.0".into()),
                private_port: 3000,
                public_port: Some(32768),
                typ: Some(PortSummaryTypeEnum::TCP),
            }]),
            ..ContainerSummary::default()
        };
        let c = container(s);
        assert_eq!(c.name, "web-1");
        assert_eq!(c.created, 42);
        assert_eq!(c.ports[0].host_port, Some(32768));
        assert!(c.ports[0].tcp);
    }
}
