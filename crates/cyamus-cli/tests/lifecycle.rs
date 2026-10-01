//! workspace-lifecycle spec: resolution, ordering, environment, exit codes.

mod common;

use std::fs;

use common::{Env, stderr};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn configured() -> (Env, std::path::PathBuf, std::path::PathBuf) {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let wt = env.add_worktree(&repo, "feature/My-Thing");
    (env, repo, wt)
}

#[test]
fn resolves_from_subdirectory() {
    let (env, _repo, wt) = configured();
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['pwd > \"$CYAMUS_WORKSPACE/pwd.out\"']\n",
    );
    let sub = wt.join("src/lib");
    fs::create_dir_all(&sub).unwrap();
    env.cyamus()
        .current_dir(&sub)
        .args(["workspace", "setup"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(wt.join("pwd.out")).unwrap().trim(),
        wt.to_str().unwrap()
    );
}

#[test]
fn resolves_from_explicit_path() {
    let (env, _repo, wt) = configured();
    env.write_manifest("myproj", "[[hooks.on_setup]]\ncommands = ['touch ran']\n");
    env.workspace("setup", &wt)
        .current_dir(&env.root)
        .assert()
        .success();
    assert!(wt.join("ran").exists());
}

#[test]
fn fails_outside_git() {
    let env = Env::new();
    let dir = env.root.join("plain");
    fs::create_dir_all(&dir).unwrap();
    env.workspace("setup", &dir)
        .assert()
        .failure()
        .stderr(contains("not inside a git worktree"));
}

#[test]
fn fails_on_missing_path() {
    let env = Env::new();
    env.workspace("setup", &env.root.join("nope"))
        .assert()
        .failure()
        .stderr(contains("does not exist"));
}

#[test]
fn fails_on_detached_head() {
    let (env, _repo, wt) = configured();
    env.git(&wt, &["checkout", "-q", "--detach"]);
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("detached HEAD"));
}

#[test]
fn unconfigured_project_is_a_notice() {
    let (env, _repo, wt) = configured();
    for action in ["setup", "teardown"] {
        env.workspace(action, &wt).assert().success().stderr(
            contains("not configured").and(contains(env.project_dir("myproj").to_str().unwrap())),
        );
    }
    assert!(!env.project_dir("myproj").exists());
}

#[test]
fn setup_is_repeatable() {
    let (env, _repo, wt) = configured();
    env.write_asset("myproj", "config.yaml", "v1\n");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'config.yaml'\ntarget = 'config.yaml'\n\
         [[hooks.on_setup]]\ncommands = ['echo run >> runs.log']\n",
    );
    env.workspace("setup", &wt).assert().success();
    fs::write(wt.join("config.yaml"), "edited\n").unwrap();
    env.workspace("setup", &wt).assert().success();
    assert_eq!(fs::read_to_string(wt.join("config.yaml")).unwrap(), "v1\n");
    assert_eq!(
        fs::read_to_string(wt.join("runs.log")).unwrap(),
        "run\nrun\n"
    );
}

#[test]
fn invalid_manifest_stops_everything() {
    let (env, _repo, wt) = configured();
    env.write_asset("myproj", "a", "a\n");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'a'\ntarget = 'a'\n[[hooks.on_setup]]\ncommands = ['touch ran']\nbogus = 1\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("cyamus.toml").and(contains("bogus")));
    assert!(!wt.join("a").exists());
    assert!(!wt.join("ran").exists());
}

#[test]
fn asset_failure_stops_hooks() {
    let (env, _repo, wt) = configured();
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'missing'\ntarget = 'x'\n[[hooks.on_setup]]\ncommands = ['touch ran']\n",
    );
    env.workspace("setup", &wt)
        .assert()
        .failure()
        .stderr(contains("missing"));
    assert!(!wt.join("ran").exists());
}

#[test]
fn environment_for_setup_and_teardown() {
    let (env, _repo, wt) = configured();
    env.write_manifest(
        "myproj",
        "[vars]\nnode-version = '22'\n\
         [[hooks.on_setup]]\ncommands = ['env | grep ^CYAMUS_ | sort > setup.env']\n\
         [[hooks.on_teardown]]\ncommands = ['env | grep ^CYAMUS_ | sort > teardown.env']\n",
    );
    env.workspace("setup", &wt).assert().success();
    env.workspace("teardown", &wt).assert().success();

    let setup = fs::read_to_string(wt.join("setup.env")).unwrap();
    let config = env.project_dir("myproj");
    let expected = [
        "CYAMUS_PROJECT=myproj".to_owned(),
        "CYAMUS_BRANCH=feature/My-Thing".to_owned(),
        "CYAMUS_BRANCH_SLUG=feature-my-thing".to_owned(),
        "CYAMUS_BRANCH_SNAKE=feature_my_thing".to_owned(),
        format!("CYAMUS_WORKSPACE={}", wt.display()),
        format!("CYAMUS_CONFIG_DIR={}", config.display()),
        format!("CYAMUS_ASSETS={}", config.join("assets").display()),
        format!("CYAMUS_BIN={}", config.join("bin").display()),
        format!("CYAMUS_CACHE_DIR={}", env.cache_dir("myproj").display()),
        "CYAMUS_EVENT=setup".to_owned(),
        "CYAMUS_VAR_NODE_VERSION=22".to_owned(),
    ];
    for line in &expected {
        assert!(
            setup.lines().any(|l| l == line),
            "missing {line} in:\n{setup}"
        );
    }
    let teardown = fs::read_to_string(wt.join("teardown.env")).unwrap();
    assert!(
        teardown.lines().any(|l| l == "CYAMUS_EVENT=teardown"),
        "{teardown}"
    );
}

#[test]
fn runtime_vars_override_manifest() {
    let (env, _repo, wt) = configured();
    env.write_manifest(
        "myproj",
        "[vars]\nenv = 'dev'\n[[hooks.on_setup]]\ncommands = ['echo \"$CYAMUS_VAR_ENV\" > env.out']\n",
    );
    env.workspace("setup", &wt)
        .args(["--var", "env=staging"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(wt.join("env.out")).unwrap(), "staging\n");
}

#[test]
fn teardown_never_deletes() {
    let (env, repo, wt) = configured();
    env.write_asset("myproj", "a.txt", "a\n");
    env.write_asset("myproj", "l.txt", "l\n");
    env.write_manifest(
        "myproj",
        "[[copy]]\nsource = 'a.txt'\ntarget = 'a.txt'\n[[link]]\nsource = 'l.txt'\ntarget = 'l.txt'\n\
         [[hooks.on_teardown]]\ncommands = ['touch torn']\n",
    );
    env.workspace("setup", &wt).assert().success();
    env.workspace("teardown", &wt).assert().success();
    assert!(wt.join("torn").exists());
    assert!(wt.join("a.txt").exists());
    assert!(
        fs::symlink_metadata(wt.join("l.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        env.git(&repo, &["branch", "--list", "feature/My-Thing"])
            .contains("feature/My-Thing")
    );
}

#[test]
fn teardown_hook_failure_exits_non_zero() {
    let (env, _repo, wt) = configured();
    env.write_manifest("myproj", "[[hooks.on_teardown]]\ncommands = ['exit 3']\n");
    let assert = env.workspace("teardown", &wt).assert().failure();
    let err = stderr(&assert);
    assert!(
        err.contains("on_teardown hook \"exit 3\" exited with code 3"),
        "{err}"
    );
}

// --- workspace registry (add-daemon-proxy) ---

#[test]
fn setup_registers_before_hooks() {
    let (env, _repo, wt) = configured();
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\n\
         commands = ['cp \"$XDG_STATE_HOME/cyamus/workspaces/myproj/feature-my-thing.toml\" seen.toml']\n",
    );
    env.workspace("setup", &wt).assert().success();
    let seen = fs::read_to_string(wt.join("seen.toml")).unwrap();
    assert!(seen.contains("branch = \"feature/My-Thing\""), "{seen}");
    assert!(
        seen.contains(&format!("path = \"{}\"", wt.display())),
        "{seen}"
    );
    assert_eq!(env.record("myproj", "feature-my-thing"), Some(seen));
}

#[test]
fn teardown_unregisters() {
    let (env, _repo, wt) = configured();
    env.write_manifest("myproj", "");
    env.workspace("setup", &wt).assert().success();
    assert!(env.record("myproj", "feature-my-thing").is_some());
    env.workspace("teardown", &wt).assert().success();
    assert!(env.record("myproj", "feature-my-thing").is_none());
    assert!(wt.exists());
}

#[test]
fn failed_teardown_hook_still_unregisters() {
    let (env, _repo, wt) = configured();
    env.write_manifest("myproj", "[[hooks.on_teardown]]\ncommands = ['exit 1']\n");
    env.workspace("setup", &wt).assert().success();
    env.workspace("teardown", &wt).assert().failure();
    assert!(env.record("myproj", "feature-my-thing").is_none());
}

#[test]
fn label_conflict_warns_and_keeps_first() {
    let (env, repo, first) = configured();
    env.write_manifest("myproj", "[[hooks.on_setup]]\ncommands = ['touch ran']\n");
    let a = env.add_worktree(&repo, "Feature_X");
    let b = env.add_worktree(&repo, "feature-x");
    env.workspace("setup", &a).assert().success();
    let assert = env.workspace("setup", &b).assert().success();
    let err = stderr(&assert);
    assert!(
        err.contains("warning:") && err.contains(a.to_str().unwrap()),
        "{err}"
    );
    assert!(b.join("ran").exists(), "hooks still run");
    let record = env.record("myproj", "feature-x").unwrap();
    assert!(record.contains(a.to_str().unwrap()), "{record}");
    // Re-running setup on the owner is not a conflict.
    let again = env.workspace("setup", &a).assert().success();
    assert!(!stderr(&again).contains("warning:"));
    drop(first);
}

#[test]
fn branch_without_usable_label_warns() {
    let (env, repo, _wt) = configured();
    env.write_manifest("myproj", "[[hooks.on_setup]]\ncommands = ['touch ran']\n");
    let wt = env.add_worktree(&repo, "___");
    let assert = env.workspace("setup", &wt).assert().success();
    assert!(stderr(&assert).contains("gets no routes"));
    assert!(wt.join("ran").exists());
    assert!(!env.state_home.join("cyamus/workspaces/myproj").exists());
}

#[test]
fn unconfigured_project_registers_nothing() {
    let (env, _repo, wt) = configured();
    env.workspace("setup", &wt).assert().success();
    assert!(!env.state_home.join("cyamus/workspaces").exists());
}
