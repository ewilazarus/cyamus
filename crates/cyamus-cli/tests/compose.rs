//! compose-stacks spec, end to end through the binary with a fake `docker` on
//! PATH that records every call and answers from files in its state dir.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Env, stderr};

/// The fake: logs `cwd|argv` per call (plus the CYAMUS_/COMPOSE_ env for
/// compose calls that aren't introspection) and plays back canned answers.
const FAKE_DOCKER: &str = r#"#!/bin/sh
S="$FAKE_DOCKER_STATE"
printf '%s|%s\n' "$PWD" "$*" >> "$S/log"
case "$*" in
  "compose version --short") cat "$S/version" 2>/dev/null || echo 5.1.2; exit 0;;
esac
project=""; prev=""
for a in "$@"; do [ "$prev" = "-p" ] && project="$a"; prev="$a"; done
case "$*" in
  *" config --format json") cat "$S/config-$project.json"; exit 0;;
  *" ps --status running --services") cat "$S/running-$project" 2>/dev/null; exit 0;;
  "ps --filter"*) cat "$S/containers" 2>/dev/null; exit 0;;
  "network inspect "*" --format "*) cat "$S/net-$3" 2>/dev/null; exit 0;;
  "network inspect "*) [ -e "$S/net-$3" ]; exit $?;;
  "network create "*) for n; do :; done; touch "$S/net-$n"; exit 0;;
  "network ls "*) for f in "$S"/net-*; do [ -e "$f" ] && basename "$f" | sed 's/^net-//'; done; exit 0;;
  "network connect "*) for n; do :; done; echo "$n" >> "$S/net-$5"; exit 0;;
  "network disconnect "*) exit 0;;
  "network rm "*) rm -f "$S/net-$3"; exit 0;;
esac
env | grep -E '^(CYAMUS_|COMPOSE_)' | sort | sed 's/^/  env /' >> "$S/log"
if [ "$project" = "$SHARED_PROJECT" ] && [ -e "$S/shared-fails" ]; then exit 1; fi
exit "${FAKE_EXIT:-0}"
"#;

struct Fake {
    env: Env,
    state: PathBuf,
    bin: PathBuf,
    wt: PathBuf,
}

fn model(services: &[(&str, &str)]) -> String {
    let body: Vec<String> = services
        .iter()
        .map(|(name, extra)| format!("\"{name}\": {{\"networks\": {{\"default\": null}}{extra}}}"))
        .collect();
    format!("{{\"services\": {{{}}}}}", body.join(", "))
}

impl Fake {
    fn new() -> Self {
        let env = Env::new();
        let state = env.root.join("fake-docker");
        let bin = env.root.join("fake-bin");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let docker = bin.join("docker");
        fs::write(&docker, FAKE_DOCKER).unwrap();
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
        let repo = env.regular_repo("repo");
        env.set_project(&repo, "myproj");
        let wt = env.add_worktree(&repo, "feat/x");
        env.write_manifest("myproj", "[vars]\nnode-version = \"22\"\n");
        fs::write(wt.join("compose.yaml"), "services: {}\n").unwrap();
        let fake = Self {
            env,
            state,
            bin,
            wt,
        };
        fake.set_model("myproj-feat-x", &[("web", ""), ("db", "")]);
        fake
    }

    fn set_model(&self, project: &str, services: &[(&str, &str)]) {
        fs::write(
            self.state.join(format!("config-{project}.json")),
            model(services),
        )
        .unwrap();
    }

    fn shared(&self, services: &[(&str, &str)]) -> PathBuf {
        let file = self.env.project_dir("myproj").join("compose.yaml");
        fs::write(&file, "services: {}\n").unwrap();
        self.set_model("myproj", services);
        file
    }

    fn cyamus(&self, dir: &Path, args: &[&str]) -> assert_cmd::Command {
        let mut cmd = self.env.cyamus();
        let path = format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap());
        cmd.current_dir(dir)
            .env("PATH", path)
            .env("FAKE_DOCKER_STATE", &self.state)
            .env("SHARED_PROJECT", "myproj")
            .env_remove("COMPOSE_FILE")
            .args(args);
        cmd
    }

    fn log(&self) -> String {
        fs::read_to_string(self.state.join("log")).unwrap_or_default()
    }

    /// The `cwd|argv` lines of the main (non-introspection) compose calls.
    fn runs(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter(|l| {
                l.contains("|compose ")
                    && !l.contains("config --format")
                    && !l.contains("--services")
                    && !l.contains("version")
            })
            .map(str::to_owned)
            .collect()
    }

    /// Env lines logged right after the `n`th main compose call.
    fn env_of(&self, n: usize) -> Vec<String> {
        let log = self.log();
        let mut seen = 0;
        let mut out = Vec::new();
        let mut capture = false;
        for line in log.lines() {
            if let Some(e) = line.strip_prefix("  env ") {
                if capture {
                    out.push(e.to_owned());
                }
                continue;
            }
            let main = line.contains("|compose ")
                && !line.contains("config --format")
                && !line.contains("--services")
                && !line.contains("version");
            capture = main && {
                seen += 1;
                seen == n + 1
            };
        }
        out
    }
}

#[test]
fn shared_requires_its_file() {
    let f = Fake::new();
    let assert = f
        .cyamus(&f.wt, &["compose-shared", "up", "-d"])
        .assert()
        .failure();
    let err = stderr(&assert);
    assert!(
        err.contains("no shared compose file at") && err.contains("projects/myproj/compose.yaml"),
        "{err}"
    );
}

#[test]
fn shared_runs_in_config_dir_with_project_env_and_exit_code() {
    let f = Fake::new();
    let file = f.shared(&[("db", "")]);
    f.cyamus(&f.wt, &["compose-shared", "up", "-d"])
        .env("CYAMUS_VAR_NODE_VERSION", "24")
        .env("CYAMUS_BRANCH", "leaked")
        .env("FAKE_EXIT", "17")
        .assert()
        .code(17);
    let runs = f.runs();
    let config = f.env.project_dir("myproj");
    assert_eq!(
        runs,
        [format!(
            "{}|compose -p myproj -f {} up -d",
            config.display(),
            file.display()
        )]
    );
    let env = f.env_of(0);
    assert!(
        env.contains(&"CYAMUS_DOMAIN=myproj.localhost".to_owned()),
        "{env:?}"
    );
    assert!(
        env.contains(&"CYAMUS_VAR_NODE_VERSION=22".to_owned()),
        "manifest wins: {env:?}"
    );
    assert!(
        !env.iter()
            .any(|e| e.starts_with("CYAMUS_BRANCH") || e.starts_with("CYAMUS_WORKSPACE")),
        "{env:?}"
    );
}

#[test]
fn worktree_without_shared_stack() {
    let f = Fake::new();
    let sub = f.wt.join("src/app");
    fs::create_dir_all(&sub).unwrap();
    f.cyamus(&sub, &["compose", "up", "-d"]).assert().success();
    assert_eq!(
        f.runs(),
        [format!(
            "{}|compose -p myproj-feat-x -f {} up -d",
            sub.display(),
            f.wt.join("compose.yaml").display()
        )]
    );
    let env = f.env_of(0);
    for expected in [
        "CYAMUS_DOMAIN=feat-x.myproj.localhost",
        "CYAMUS_VAR_NODE_VERSION=22",
        "CYAMUS_BRANCH=feat/x",
        "CYAMUS_URL_SUFFIX=:1355",
    ] {
        assert!(
            env.contains(&expected.to_owned()),
            "{expected} missing: {env:?}"
        );
    }
    assert!(
        !f.log().contains("network create"),
        "no shared stack, no network"
    );
}

#[test]
fn worktree_keeps_inherited_runtime_vars() {
    let f = Fake::new();
    f.cyamus(&f.wt, &["compose", "up", "-d"])
        .env("CYAMUS_VAR_NODE_VERSION", "24")
        .assert()
        .success();
    assert!(
        f.env_of(0)
            .contains(&"CYAMUS_VAR_NODE_VERSION=24".to_owned())
    );
}

#[test]
fn explicit_files_are_kept() {
    let f = Fake::new();
    fs::write(f.wt.join("other.yaml"), "services: {}\n").unwrap();
    f.cyamus(&f.wt, &["compose", "-f", "other.yaml", "ps"])
        .assert()
        .success();
    assert_eq!(
        f.runs(),
        [format!(
            "{}|compose -p myproj-feat-x -f other.yaml ps",
            f.wt.display()
        )]
    );
}

#[test]
fn no_compose_file_is_an_error() {
    let f = Fake::new();
    fs::remove_file(f.wt.join("compose.yaml")).unwrap();
    let assert = f.cyamus(&f.wt, &["compose", "ps"]).assert().failure();
    assert!(stderr(&assert).contains("no compose file found"));
}

#[test]
fn shadowing_starts_shared_stack_then_worktree() {
    let f = Fake::new();
    let shared = f.shared(&[("db", ""), ("mailpit", "")]);
    fs::write(f.state.join("containers"), "c1 db\nc2 mailpit\n").unwrap();
    let assert = f.cyamus(&f.wt, &["compose", "up", "-d"]).assert().success();
    let err = stderr(&assert);
    assert!(err.contains("using shared: db"), "{err}");
    assert!(
        err.contains("starting the shared stack for myproj"),
        "{err}"
    );

    let runs = f.runs();
    let config = f.env.project_dir("myproj");
    assert_eq!(
        runs[0],
        format!(
            "{}|compose -p myproj -f {} up -d",
            config.display(),
            shared.display()
        )
    );
    let override_file = f.env.state_home.join("cyamus/compose/myproj/feat-x.yaml");
    assert_eq!(
        runs[1],
        format!(
            "{}|compose -p myproj-feat-x -f {} -f {} up -d",
            f.wt.display(),
            f.wt.join("compose.yaml").display(),
            override_file.display()
        )
    );
    let text = fs::read_to_string(&override_file).unwrap();
    assert!(
        text.contains("\"db\":\n    profiles: [\"cyamus-shadowed\"]"),
        "{text}"
    );
    assert!(text.contains("\"name\":\"cyamus-myproj-feat-x\""), "{text}");

    // Network created and shared containers attached before the worktree up.
    let log = f.log();
    let created = log.find("network create").unwrap();
    let connected = log
        .find("network connect --alias db cyamus-myproj-feat-x c1")
        .unwrap();
    let worktree_up = log
        .find(&format!("-f {} up -d", override_file.display()))
        .unwrap();
    assert!(created < connected && connected < worktree_up, "{log}");
    assert!(
        log.contains("--label dev.cyamus.project=myproj --label dev.cyamus.workspace=feat-x"),
        "{log}"
    );
}

#[test]
fn running_shared_stack_is_left_alone() {
    let f = Fake::new();
    f.shared(&[("db", ""), ("debug", ", \"profiles\": [\"debug\"]")]);
    fs::write(f.state.join("running-myproj"), "db\n").unwrap();
    let assert = f.cyamus(&f.wt, &["compose", "up", "-d"]).assert().success();
    assert!(!stderr(&assert).contains("starting the shared stack"));
    assert_eq!(f.runs().len(), 1, "only the worktree stack: {:?}", f.runs());
}

#[test]
fn failing_shared_stack_stops_everything() {
    let f = Fake::new();
    f.shared(&[("db", "")]);
    fs::write(f.state.join("shared-fails"), "").unwrap();
    let assert = f.cyamus(&f.wt, &["compose", "up", "-d"]).assert().failure();
    assert!(stderr(&assert).contains("the shared stack failed to start"));
    assert!(
        !f.runs().iter().any(|r| r.contains("myproj-feat-x")),
        "{:?}",
        f.runs()
    );
}

#[test]
fn down_leaves_shared_stack_and_removes_network() {
    let f = Fake::new();
    f.shared(&[("db", "")]);
    fs::write(f.state.join("net-cyamus-myproj-feat-x"), "").unwrap();
    f.cyamus(&f.wt, &["compose", "down"]).assert().success();
    let runs = f.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].contains("compose -p myproj-feat-x") && runs[0].ends_with(" down"));
    assert!(f.log().contains("network rm cyamus-myproj-feat-x"));
    assert!(!f.state.join("net-cyamus-myproj-feat-x").exists());
}

#[test]
fn old_compose_is_rejected() {
    let f = Fake::new();
    fs::write(f.state.join("version"), "2.20.2\n").unwrap();
    let assert = f.cyamus(&f.wt, &["compose", "up"]).assert().failure();
    assert!(stderr(&assert).contains("Docker Compose 2.24.4 or later is required (found 2.20.2)"));
}
