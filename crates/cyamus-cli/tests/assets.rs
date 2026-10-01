//! workspace-assets spec: links, copies, exclusion, override mode.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use common::{Env, status};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn setup() -> (Env, PathBuf, PathBuf) {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let wt = env.add_worktree(&repo, "feat/x");
    (env, repo, wt)
}

#[test]
fn escaping_target_applies_nothing() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "a", "a");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'a'\ntarget = 'a'\n[[link]]\nsource = 'a'\ntarget = '../../etc/foo'\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("../../etc/foo"));
    assert!(!wt.join("a").exists());
}

#[test]
fn missing_source_fails() {
    let (env, _repo, wt) = setup();
    env.write_manifest("myproj", "[[copy]]\nsource = 'nope.yaml'\ntarget = 'x'\n");
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("nope.yaml"));
}

#[test]
fn creates_link_with_parents_and_is_idempotent() {
    let (env, _repo, wt) = setup();
    let source = env.write_asset("myproj", "env.local", "SECRET=1\n");
    env.write_manifest(
        "myproj",
        "[[link]]\nsource = 'env.local'\ntarget = 'config/.env.local'\n",
    );
    env.workspace("setup", &wt).assert().success();
    let target = wt.join("config/.env.local");
    assert_eq!(fs::read_link(&target).unwrap(), source);
    env.workspace("setup", &wt).assert().success();
    assert_eq!(fs::read_link(&target).unwrap(), source);
}

#[test]
fn link_conflicts_are_errors() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "env.local", "x");
    env.write_manifest(
        "myproj",
        "[[link]]\nsource = 'env.local'\ntarget = '.env.local'\n",
    );

    fs::write(wt.join(".env.local"), "mine").unwrap();
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains(".env.local"));
    assert_eq!(fs::read_to_string(wt.join(".env.local")).unwrap(), "mine");

    fs::remove_file(wt.join(".env.local")).unwrap();
    std::os::unix::fs::symlink("/elsewhere", wt.join(".env.local")).unwrap();
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains(".env.local"));
    assert_eq!(
        fs::read_link(wt.join(".env.local")).unwrap(),
        PathBuf::from("/elsewhere")
    );
}

#[test]
fn copies_files_and_directories_preserving_permissions() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "config.local.yaml", "a: 1\n");
    env.write_asset("myproj", "vscode/settings.json", "{}\n");
    let script = env.write_asset("myproj", "vscode/sub/run.sh", "#!/bin/sh\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'config.local.yaml'\ntarget = 'config.local.yaml'\n\
         [[copy]]\nsource = 'vscode'\ntarget = 'editor'\n",
    );
    env.workspace("setup", &wt).assert().success();

    let copied = wt.join("config.local.yaml");
    assert!(
        !fs::symlink_metadata(&copied)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&copied).unwrap(), "a: 1\n");
    assert_eq!(
        fs::read_to_string(wt.join("editor/settings.json")).unwrap(),
        "{}\n"
    );
    let mode = fs::metadata(wt.join("editor/sub/run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn overwrite_control() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "a", "fresh");
    env.write_asset("myproj", "b", "fresh");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'a'\ntarget = 'a'\n\
         [[copy]]\nsource = 'b'\ntarget = 'b'\noverwrite = false\n",
    );
    env.workspace("setup", &wt).assert().success();
    fs::write(wt.join("a"), "edited").unwrap();
    fs::write(wt.join("b"), "edited").unwrap();
    env.workspace("setup", &wt).assert().success();
    assert_eq!(fs::read_to_string(wt.join("a")).unwrap(), "fresh");
    assert_eq!(fs::read_to_string(wt.join("b")).unwrap(), "edited");
}

#[test]
fn non_override_targets_are_excluded_and_user_lines_kept() {
    let (env, repo, wt) = setup();
    let exclude = repo.join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    fs::write(&exclude, "# mine\n*.log\n").unwrap();
    env.write_asset("myproj", "env.local", "x");
    env.write_asset("myproj", "c.yaml", "x");
    env.write_manifest(
        "myproj",
        "[[link]]\nsource = 'env.local'\ntarget = '.env.local'\n\
         [[copy]]\nsource = 'c.yaml'\ntarget = 'conf/c.yaml'\n",
    );
    env.workspace("setup", &wt).assert().success();

    assert_eq!(status(&env, &wt), "");
    let content = fs::read_to_string(&exclude).unwrap();
    assert!(content.starts_with("# mine\n*.log\n"), "{content}");
    assert!(
        content.contains("/.env.local\n") && content.contains("/conf/c.yaml\n"),
        "{content}"
    );

    // Re-running does not duplicate the block.
    env.workspace("setup", &wt).assert().success();
    assert_eq!(fs::read_to_string(&exclude).unwrap(), content);
}

#[test]
fn concurrent_setups_leave_a_well_formed_block() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    env.write_asset("myproj", "env.local", "x");
    env.write_manifest(
        "myproj",
        "[[link]]\nsource = 'env.local'\ntarget = '.env.local'\n",
    );
    let worktrees: Vec<_> = (0..6)
        .map(|i| env.add_worktree(&repo, &format!("b{i}")))
        .collect();

    let children: Vec<_> = worktrees
        .iter()
        .map(|wt| {
            let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cyamus"));
            cmd.args(["workspace", "setup"])
                .arg(wt)
                .env("HOME", &env.home)
                .env("XDG_CONFIG_HOME", &env.config_home)
                .env("XDG_CACHE_HOME", &env.cache_home)
                .env("GIT_CONFIG_NOSYSTEM", "1");
            cmd.spawn().unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    let content = fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    assert_eq!(
        content.matches("# >>> cyamus managed >>>").count(),
        1,
        "{content}"
    );
    assert_eq!(
        content.matches("# <<< cyamus managed <<<").count(),
        1,
        "{content}"
    );
    assert_eq!(content.matches("/.env.local").count(), 1, "{content}");
    for wt in &worktrees {
        assert_eq!(status(&env, wt), "");
    }
}

#[test]
fn override_replaces_tracked_file() {
    let (env, _repo, wt) = setup();
    let source = env.write_asset("myproj", "vscode-settings.json", "{\"mine\": true}\n");
    env.write_manifest(
        "myproj",
        "[[link]]\nsource = 'vscode-settings.json'\ntarget = '.vscode/settings.json'\noverride = true\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        fs::read_link(wt.join(".vscode/settings.json")).unwrap(),
        source
    );
    assert_eq!(status(&env, &wt), "");
    let exclude = fs::read_to_string(env.root.join("repo/.git/info/exclude")).unwrap_or_default();
    assert!(!exclude.contains("settings.json"), "{exclude}");
    // Repeatable.
    env.workspace("setup", &wt).assert().success();
    assert_eq!(status(&env, &wt), "");
}

#[test]
fn override_copy_of_tracked_file() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "tracked.txt", "replaced\n");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'tracked.txt'\ntarget = 'tracked.txt'\noverride = true\n",
    );
    env.workspace("setup", &wt).assert().success();
    assert_eq!(
        fs::read_to_string(wt.join("tracked.txt")).unwrap(),
        "replaced\n"
    );
    assert_eq!(status(&env, &wt), "");
}

#[test]
fn override_of_untracked_target_fails() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "a", "x");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'a'\ntarget = 'untracked'\noverride = true\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("not tracked").and(contains("override")));
}

#[test]
fn non_override_asset_on_tracked_file_fails() {
    let (env, _repo, wt) = setup();
    env.write_asset("myproj", "a", "x");
    env.write_manifest("myproj", "[[copy]]\nsource = 'a'\ntarget = 'tracked.txt'\n");
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("is tracked").and(contains("override = true")));
    assert_eq!(
        fs::read_to_string(wt.join("tracked.txt")).unwrap(),
        "tracked\n"
    );
}
