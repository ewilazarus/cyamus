//! docker-discovery against a real Docker daemon. Opt-in: set
//! `CYAMUS_DOCKER_TESTS=1` (CI runners without Docker skip it).

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cyamus_daemon::docker;
use cyamus_daemon::state::State;
use cyamus_registry::{Record, Registry};

fn enabled() -> bool {
    std::env::var_os("CYAMUS_DOCKER_TESTS").is_some_and(|v| v == "1")
}

fn docker_cli(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "docker {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

async fn wait_for(state: &State, what: &str, pred: impl Fn(&State) -> bool) -> Duration {
    let start = Instant::now();
    while !pred(state) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    start.elapsed()
}

#[tokio::test]
async fn container_routes_follow_docker_events() {
    if !enabled() {
        eprintln!("skipped: set CYAMUS_DOCKER_TESTS=1 to run against Docker");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let registry = Registry::new(root.join("workspaces"), root.join("registry.lock"));
    registry
        .register(&Record {
            project: "cyamustest".into(),
            branch: "dock".into(),
            label: "dock".into(),
            path: wt.clone(),
        })
        .unwrap();
    let endpoint = docker::resolve_endpoint(|k| std::env::var_os(k));
    let state = Arc::new(State::new(
        registry,
        root.join("projects"),
        endpoint.clone(),
        1355,
    ));
    tokio::spawn(docker::watch(endpoint, Arc::clone(&state)));
    wait_for(&state, "docker connection", State::docker_reachable).await;

    let name = format!("cyamus-test-{}", std::process::id());
    let _cleanup = Cleanup(name.clone());
    let host = format!("{name}.dock.cyamustest.localhost");
    docker_cli(&[
        "run",
        "-d",
        "--rm",
        "--name",
        &name,
        "-p",
        "127.0.0.1::80",
        "--label",
        "cyamus.project=cyamustest",
        "--label",
        "cyamus.workspace=dock",
        "alpine:3",
        "sleep",
        "60",
    ]);
    let took = wait_for(&state, "route", |s| s.table().get(&host).is_some()).await;
    assert!(
        took < Duration::from_secs(2),
        "route appeared after {took:?}"
    );
    let target = state.table().get(&host).unwrap().target;
    let published = docker_cli(&["port", &name, "80/tcp"]);
    assert_eq!(published, target.to_string());

    docker_cli(&["rm", "-f", &name]);
    let took = wait_for(&state, "route removal", |s| s.table().get(&host).is_none()).await;
    assert!(
        took < Duration::from_secs(2),
        "route disappeared after {took:?}"
    );
}
