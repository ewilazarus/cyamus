//! workspace-fingerprints spec: fingerprint variables and the cache directory.

mod common;

use std::fs;

use common::Env;

const MANIFEST: &str = "\
[[copy]]
source = 'config.local.yaml'
target = 'config.local.yaml'

[[fingerprint]]
name = 'docker-deps'
files = ['config.local.yaml', 'Dockerfile']

[[hooks.on_setup]]
commands = ['echo \"$CYAMUS_FINGERPRINT_DOCKER_DEPS\" > fp.out']

[[hooks.on_teardown]]
commands = ['echo \"$CYAMUS_FINGERPRINT_DOCKER_DEPS\" > fp-teardown.out']
";

#[test]
fn fingerprint_covers_copied_assets_and_tracks_changes() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    env.write_asset("myproj", "config.local.yaml", "a: 1\n");
    env.write_manifest("myproj", MANIFEST);
    let a = env.add_worktree(&repo, "a");
    let b = env.add_worktree(&repo, "b");

    env.workspace("setup", &a).assert().success();
    env.workspace("setup", &b).assert().success();
    let fp_a = fs::read_to_string(a.join("fp.out")).unwrap();
    let fp_b = fs::read_to_string(b.join("fp.out")).unwrap();
    assert_eq!(fp_a.trim().len(), 12);
    assert_eq!(fp_a, fp_b, "identical inputs give identical fingerprints");

    // A fingerprint computed without the copied file would differ.
    let c = env.add_worktree(&repo, "c");
    env.write_manifest(
        "myproj",
        &MANIFEST.replace(
            "[[copy]]\nsource = 'config.local.yaml'\ntarget = 'config.local.yaml'\n",
            "",
        ),
    );
    env.workspace("setup", &c).assert().success();
    assert_ne!(fs::read_to_string(c.join("fp.out")).unwrap(), fp_a);

    // Changing an input changes the value; teardown sees it too.
    env.write_manifest("myproj", MANIFEST);
    fs::write(b.join("Dockerfile"), "FROM scratch\n").unwrap();
    env.workspace("teardown", &b).assert().success();
    assert_ne!(fs::read_to_string(b.join("fp-teardown.out")).unwrap(), fp_a);
}

#[test]
fn cache_dir_is_created_and_outlives_worktrees() {
    let env = Env::new();
    let repo = env.regular_repo("repo");
    env.set_project(&repo, "myproj");
    env.write_manifest(
        "myproj",
        "[[hooks.on_setup]]\ncommands = ['test -d \"$CYAMUS_CACHE_DIR\"', \
         'if [ -f \"$CYAMUS_CACHE_DIR/stamp\" ]; then cp \"$CYAMUS_CACHE_DIR/stamp\" seen; else echo first > \"$CYAMUS_CACHE_DIR/stamp\"; fi']\n",
    );
    assert!(!env.cache_dir("myproj").exists());

    let first = env.add_worktree(&repo, "first");
    env.workspace("setup", &first).assert().success();
    assert!(env.cache_dir("myproj").join("stamp").exists());
    env.workspace("teardown", &first).assert().success();
    env.git(
        &repo,
        &["worktree", "remove", "--force", first.to_str().unwrap()],
    );

    let second = env.add_worktree(&repo, "second");
    env.workspace("setup", &second).assert().success();
    assert_eq!(fs::read_to_string(second.join("seen")).unwrap(), "first\n");
}
