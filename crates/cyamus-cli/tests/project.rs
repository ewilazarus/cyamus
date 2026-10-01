//! project-config spec: identity, guessing, validation.

mod common;

use common::{Env, stderr};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

#[test]
fn reads_key_from_regular_checkout() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let wt = env.add_worktree(&repo, "feat/x");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['echo \"$CYAMUS_PROJECT\" > project.out']\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        std::fs::read_to_string(wt.join("project.out")).unwrap(),
        "myproj\n"
    );
}

#[test]
fn reads_key_from_bare_checkout() {
    let env = Env::new();
    let bare = env.bare_repo("widgets");
    env.set_project(&bare, "myproj");
    let wt = env.add_worktree(&bare, "feat/x");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['echo \"$CYAMUS_PROJECT\" > project.out']\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        std::fs::read_to_string(wt.join("project.out")).unwrap(),
        "myproj\n"
    );
}

#[test]
fn guesses_from_origin_and_persists() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.git(
        &repo,
        &["remote", "add", "origin", "git@github.com:acme/My_Proj.git"],
    );
    let wt = env.add_worktree(&repo, "feat/x");

    let assert = env.workspace("setup", &wt).assert().success();
    assert!(
        stderr(&assert).contains("guessed project name \"my-proj\""),
        "{}",
        stderr(&assert)
    );
    assert_eq!(
        env.git(&repo, &["config", "--local", "cyamus.project"]),
        "my-proj"
    );

    // Another worktree reads the persisted key: no guess is reported.
    let wt2 = env.add_worktree(&repo, "feat/y");
    let assert = env.workspace("setup", &wt2).assert().success();
    assert!(!stderr(&assert).contains("guessed"), "{}", stderr(&assert));
}

#[test]
fn guesses_from_bare_dir_without_origin() {
    let env = Env::new();
    let bare = env.bare_repo("widgets");
    let wt = env.add_worktree(&bare, "feat/x");
    env.workspace("setup", &wt)
        .assert()
        .success()
        .stderr(contains("guessed project name \"widgets\""));
    assert_eq!(
        env.git(&bare, &["config", "--local", "cyamus.project"]),
        "widgets"
    );
}

#[test]
fn guesses_from_regular_dir_without_origin() {
    let env = Env::new();
    let repo = env.regular_repo("Gadgets");
    let wt = env.add_worktree(&repo, "feat/x");
    env.workspace("setup", &wt)
        .assert()
        .success()
        .stderr(contains("guessed project name \"gadgets\""));
}

#[test]
fn rejects_invalid_configured_name() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "My Proj!");
    env.workspace("setup", &repo)
        .assert()
        .failure()
        .stderr(contains("My Proj!").and(contains("lowercase letters, digits and hyphens")));
}

#[test]
fn parallel_first_setups_agree() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.git(
        &repo,
        &["remote", "add", "origin", "git@github.com:acme/racer.git"],
    );
    let worktrees: Vec<_> = (0..6)
        .map(|i| env.add_worktree(&repo, &format!("b{i}")))
        .collect();
    let children: Vec<_> = worktrees
        .iter()
        .map(|wt| {
            std::process::Command::new(assert_cmd::cargo::cargo_bin!("cyamus"))
                .args(["workspace", "setup"])
                .arg(wt)
                .env("HOME", &env.home)
                .env("XDG_CONFIG_HOME", &env.config_home)
                .env("XDG_CACHE_HOME", &env.cache_home)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(
        env.git(&repo, &["config", "--local", "--get-all", "cyamus.project"]),
        "racer"
    );
}
