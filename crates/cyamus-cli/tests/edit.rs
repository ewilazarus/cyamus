//! project-config spec: `cyamus edit`.

mod common;

use std::fs;
use std::path::PathBuf;

use common::Env;
use predicates::str::contains;

/// An EDITOR stub that records the argument it was launched with.
fn stub_editor(env: &Env) -> (PathBuf, PathBuf) {
    let record = env.root.join("editor.arg");
    let script = env.root.join("editor.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", record.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, record)
}

#[test]
fn opens_existing_config_directory() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let wt = env.add_worktree(&repo, "feat/x");
    env.write_manifest("myproj", "version = 1\n[vars]\nkeep = 'me'\n");
    let (editor, record) = stub_editor(&env);

    env.cyamus()
        .current_dir(&wt)
        .arg("edit")
        .env("EDITOR", &editor)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&record).unwrap(),
        env.project_dir("myproj").to_str().unwrap()
    );
    // Existing manifest untouched.
    assert!(
        fs::read_to_string(env.project_dir("myproj").join("cyamus.toml"))
            .unwrap()
            .contains("keep")
    );
}

#[test]
fn scaffolds_missing_config_directory() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let (editor, record) = stub_editor(&env);

    env.cyamus()
        .args(["edit"])
        .arg(&repo)
        .env("EDITOR", &editor)
        .assert()
        .success()
        .stderr(contains("creating config directory"));
    let dir = env.project_dir("myproj");
    assert_eq!(fs::read_to_string(&record).unwrap(), dir.to_str().unwrap());
    assert!(dir.join("assets").is_dir());
    assert!(dir.join("bin").is_dir());
    assert_eq!(
        fs::read_to_string(dir.join("cyamus.toml")).unwrap(),
        "version = 1\n"
    );

    // The scaffolded manifest is valid: setup now succeeds and does nothing.
    env.workspace("setup", &repo).assert().success();
}

#[test]
fn editor_may_carry_arguments() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    let record = env.root.join("editor.args");
    let script = env.root.join("ed.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho \"$@\" > '{}'\n", record.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    env.cyamus()
        .arg("edit")
        .arg(&repo)
        .env("EDITOR", format!("{} -w", script.display()))
        .assert()
        .success();
    let args = fs::read_to_string(&record).unwrap();
    assert_eq!(
        args.trim(),
        format!("-w {}", env.project_dir("myproj").display())
    );
}

#[test]
fn editor_unset_fails_and_creates_nothing() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    env.cyamus()
        .arg("edit")
        .arg(&repo)
        .assert()
        .failure()
        .stderr(contains("EDITOR"));
    env.cyamus()
        .arg("edit")
        .arg(&repo)
        .env("EDITOR", "")
        .assert()
        .failure()
        .stderr(contains("EDITOR"));
    assert!(!env.project_dir("myproj").exists());
}
