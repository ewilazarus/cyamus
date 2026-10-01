//! workspace-hooks spec: selection, resolution, execution.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{Env, stderr, stdout};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn setup(branch: &str) -> (Env, PathBuf) {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let wt = env.add_worktree(&repo, branch);
    (env, wt)
}

#[test]
fn conditions_select_groups() {
    let (env, wt) = setup("gabriel/wip-1");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['echo always >> log']\n\
         [[hooks.on_setup]]\nconditions = { if_branch_matches = 'gabriel/*' }\ncommands = ['echo mine >> log']\n\
         [[hooks.on_setup]]\nconditions = { if_branch_matches = 'gabriel/*', if_branch_not_matches = 'gabriel/wip-*' }\ncommands = ['echo not-wip >> log']\n\
         [[hooks.on_setup]]\nconditions = { if_branch_not_matches = 'gabriel/*' }\ncommands = ['echo theirs >> log']\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        fs::read_to_string(wt.join("log")).unwrap(),
        "always\nmine\n"
    );
}

#[test]
fn bin_scripts_and_inline_commands() {
    let (env, wt) = setup("main2");
    env.write_bin(
        "myproj",
        "install_deps",
        "#!/bin/sh\necho \"bin $CYAMUS_EVENT\" >> log\n",
        true,
    );
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['install_deps', 'echo inline >> log']\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        fs::read_to_string(wt.join("log")).unwrap(),
        "bin setup\ninline\n"
    );
}

#[test]
fn non_executable_bin_script_fails() {
    let (env, wt) = setup("main2");
    env.write_bin("myproj", "install_deps", "#!/bin/sh\n", false);
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['install_deps']\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("install_deps").and(contains("not executable")));
}

#[test]
fn runs_in_worktree_root_and_streams_output() {
    let (env, wt) = setup("main2");
    env.write_manifest("myproj", "[[hooks.on_setup]]\ncommands = ['pwd']\n");
    let sub = wt.join("deep/dir");
    fs::create_dir_all(&sub).unwrap();
    let assert = env.workspace("setup", &sub).assert().success();
    assert_eq!(stdout(&assert).trim(), wt.to_str().unwrap());
    assert!(
        stderr(&assert).contains("▸ on_setup: pwd"),
        "{}",
        stderr(&assert)
    );
}

#[test]
fn failure_stops_remaining_hooks() {
    let (env, wt) = setup("main2");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['echo one >> log', 'exit 1', 'echo three >> log']\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("on_setup hook \"exit 1\" exited with code 1"));
    assert_eq!(fs::read_to_string(wt.join("log")).unwrap(), "one\n");
}

#[test]
fn no_matching_hooks_succeeds() {
    let (env, wt) = setup("main2");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\nconditions = { if_branch_matches = 'release/*' }\ncommands = ['exit 1']\n",
    );
    env.workspace("setup", &wt).assert().success();
    env.workspace("teardown", &wt).assert().success();
}
