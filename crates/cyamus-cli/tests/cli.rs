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
