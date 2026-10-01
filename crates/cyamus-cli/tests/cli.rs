mod common;

use common::Env;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

#[test]
fn help_lists_commands() {
    let env = Env::new();
    env.cyamus()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("workspace").and(contains("edit")));
    env.cyamus()
        .args(["workspace", "--help"])
        .assert()
        .success()
        .stdout(contains("setup").and(contains("teardown")));
}

#[test]
fn malformed_runtime_var_fails_before_resolving() {
    let env = Env::new();
    // The path does not exist: the error must be about --var, not the path.
    env.cyamus()
        .args(["workspace", "setup", "/does/not/exist", "--var", "novalue"])
        .assert()
        .failure()
        .stderr(contains("KEY=VALUE"));
}

#[test]
fn closed_stdout_exits_quietly() {
    // `cyamus … | head` closes the pipe early; that must not be a panic.
    let env = common::Env::new();
    let mut child = env
        .std_cyamus()
        .args(["daemon", "install", "--dry-run"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(out.status.code(), Some(141), "{stderr}");
}
