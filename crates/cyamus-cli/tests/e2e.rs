//! End-to-end: imitate Orca's worktree lifecycle against bare and regular
//! repositories.

mod common;

use std::fs;
use std::path::Path;

use common::{Env, status};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn orca_lifecycle(env: &Env, repo: &Path, expected_project: &str) {
    // Orca creates a worktree, then its setup script runs cyamus.
    let wt = env.add_worktree(repo, "feature/login");
    env.workspace("setup", &wt)
        .assert()
        .success()
        .stderr(contains("guessed project name").and(contains("not configured")));

    // The user configures the project.
    env.write_asset(expected_project, "env.local", "TOKEN=1\n");
    env.write_asset(expected_project, "config.yaml", "debug: true\n");
    env.write_manifest(
        expected_project,
        "[vars]\nregion = 'eu'\n\
         [[link]]\nsource = 'env.local'\ntarget = '.env.local'\n\
         [[copy]]\nsource = 'config.yaml'\ntarget = 'config/app.yaml'\n\
         [[fingerprint]]\nname = 'deps'\nfiles = ['tracked.txt']\n\
         [[hooks.on_setup]]\ncommands = ['echo \"setup $CYAMUS_BRANCH_SLUG $CYAMUS_VAR_REGION\" >> \"$CYAMUS_CACHE_DIR/events\"']\n\
         [[hooks.on_teardown]]\ncommands = ['echo \"teardown $CYAMUS_BRANCH_SLUG\" >> \"$CYAMUS_CACHE_DIR/events\"']\n",
    );

    // Re-running setup (reset) applies everything.
    env.workspace("setup", &wt).assert().success();
    assert!(
        fs::symlink_metadata(wt.join(".env.local"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(wt.join("config/app.yaml")).unwrap(),
        "debug: true\n"
    );
    assert_eq!(status(env, &wt), "");

    // Orca closes the worktree: teardown script, then Orca deletes it.
    env.workspace("teardown", &wt).assert().success();
    assert!(wt.exists());
    env.git(
        repo,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    env.git(repo, &["branch", "-D", "feature/login"]);

    let events = fs::read_to_string(env.cache_dir(expected_project).join("events")).unwrap();
    assert_eq!(events, "setup feature-login eu\nteardown feature-login\n");
}

#[test]
fn bare_repository() {
    let env = Env::new();
    let bare = env.bare_repo("widgets");
    orca_lifecycle(&env, &bare, "widgets");
}

#[test]
fn regular_repository() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/Shop_Front.git",
        ],
    );
    orca_lifecycle(&env, &repo, "shop-front");
}
