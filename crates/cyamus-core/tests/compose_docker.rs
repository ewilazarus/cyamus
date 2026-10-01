//! compose-stacks against a real Docker. Opt-in: `CYAMUS_DOCKER_TESTS=1`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use cyamus_core::compose::docker::{Docker, PROJECT_LABEL};
use cyamus_core::compose::{model, overlay};

fn enabled() -> bool {
    let on = std::env::var_os("CYAMUS_DOCKER_TESTS").is_some_and(|v| v == "1");
    if !on {
        eprintln!("skipped: set CYAMUS_DOCKER_TESTS=1 to run against Docker");
    }
    on
}

const WORKTREE: &str = r#"
services:
  web:
    image: alpine:3
    command: sleep 300
    depends_on:
      db: {condition: service_started}
      cache: {condition: service_started}
  cache:
    image: alpine:3
    command: sleep 300
  worker:
    image: alpine:3
    command: sleep 300
    networks:
      backend: {aliases: [jobs]}
  hostnet:
    image: alpine:3
    command: sleep 300
    network_mode: host
  db:
    image: alpine:3
networks:
  backend: {}
"#;

#[test]
fn generated_override_is_accepted_by_compose() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("compose.yaml"), WORKTREE).unwrap();
    let docker = Docker {
        cwd: dir.to_owned(),
        ..Docker::default()
    };
    let model = model::parse(
        &docker
            .config_json("cyt", &[dir.join("compose.yaml").into()])
            .unwrap(),
    )
    .unwrap();
    let shadowed = BTreeSet::from(["db".to_owned()]);
    let text = overlay::worktree_override(&model, &shadowed, Some("cyamus-cyt-x")).unwrap();
    fs::write(dir.join("o.yaml"), &text).unwrap();

    let merged = docker
        .config_json(
            "cyt",
            &[dir.join("compose.yaml").into(), dir.join("o.yaml").into()],
        )
        .unwrap();
    let merged = model::parse(&merged).unwrap();
    assert_eq!(merged.services["db"].profiles, ["cyamus-shadowed"]);
    let web = &merged.services["web"];
    assert_eq!(web.depends_on.keys().collect::<Vec<_>>(), ["cache"]);
    let nets = web.networks.as_ref().unwrap();
    assert!(nets.contains_key("default") && nets.contains_key("cyamus-net"));
    let worker = merged.services["worker"].networks.as_ref().unwrap();
    assert!(
        !worker.contains_key("default"),
        "explicit networks kept as-is"
    );
    assert_eq!(worker["backend"]["aliases"][0], "jobs");
    assert!(merged.services["hostnet"].networks.is_none());

    // Without activating profiles, compose would not start the shadowed db.
    let out = Command::new("docker")
        .current_dir(dir)
        .args([
            "compose",
            "-p",
            "cyt",
            "-f",
            "compose.yaml",
            "-f",
            "o.yaml",
            "config",
            "--services",
        ])
        .output()
        .unwrap();
    let services = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!services.lines().any(|s| s == "db"), "{services}");
}

fn sh(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
fn network_ensure_attach_reconcile_remove() {
    if !enabled() {
        return;
    }
    let project = format!("cyt{}", std::process::id());
    let network = format!("cyamus-{project}-feat");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("compose.yaml"),
        "services:\n  db:\n    image: alpine:3\n    command: sleep 300\n",
    )
    .unwrap();
    let docker = Docker {
        cwd: dir.to_owned(),
        ..Docker::default()
    };
    let up = |extra: &[&str]| {
        let mut args = vec!["compose", "-p", project.as_str(), "up", "-d"];
        args.extend(extra);
        assert!(
            Command::new("docker")
                .current_dir(dir)
                .args(&args)
                .status()
                .unwrap()
                .success()
        );
    };
    up(&[]);
    struct Down<'a>(&'a Path, String, String);
    impl Drop for Down<'_> {
        fn drop(&mut self) {
            let _ = Command::new("docker")
                .current_dir(self.0)
                .args(["compose", "-p", &self.1, "down", "-t", "0"])
                .output();
            let _ = Command::new("docker")
                .args(["network", "rm", &self.2])
                .output();
        }
    }
    let _down = Down(dir, project.clone(), network.clone());

    docker
        .ensure_network(&network, &[(PROJECT_LABEL, &project)])
        .unwrap();
    docker
        .ensure_network(&network, &[(PROJECT_LABEL, &project)])
        .unwrap(); // idempotent
    assert_eq!(
        docker.labelled_networks(PROJECT_LABEL, &project).unwrap(),
        std::slice::from_ref(&network)
    );

    let containers = docker.project_containers(&project).unwrap();
    assert_eq!(containers.len(), 1);
    docker.attach(&network, &containers).unwrap();
    docker.attach(&network, &containers).unwrap(); // skips existing attachments
    let aliases = sh(&[
        "inspect",
        "-f",
        &format!("{{{{json (index .NetworkSettings.Networks \"{network}\").Aliases}}}}"),
        &containers[0].0,
    ]);
    assert!(aliases.contains("\"db\""), "{aliases}");

    // A recreated container loses the attachment until reconciled.
    up(&["--force-recreate"]);
    let containers = docker.project_containers(&project).unwrap();
    for network in docker.labelled_networks(PROJECT_LABEL, &project).unwrap() {
        docker.attach(&network, &containers).unwrap();
    }
    let attached = sh(&[
        "network",
        "inspect",
        &network,
        "-f",
        "{{range $id, $c := .Containers}}{{$id}} {{end}}",
    ]);
    assert!(attached.starts_with(&containers[0].0), "{attached}");

    docker.remove_network(&network).unwrap();
    assert!(
        docker
            .labelled_networks(PROJECT_LABEL, &project)
            .unwrap()
            .is_empty()
    );
}
