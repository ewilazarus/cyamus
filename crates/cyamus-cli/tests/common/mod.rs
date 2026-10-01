//! Integration-test harness: real git repositories in a temp dir, an isolated
//! HOME/XDG environment, and helpers to drive the `cyamus` binary.

#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use tempfile::TempDir;

pub struct Env {
    _tmp: TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub cache_home: PathBuf,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize: git reports real paths (/private/var/... on macOS).
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let config_home = root.join("config");
        let cache_home = root.join("cache");
        for dir in [&home, &config_home, &cache_home] {
            fs::create_dir_all(dir).unwrap();
        }
        Self {
            _tmp: tmp,
            root,
            home,
            config_home,
            cache_home,
        }
    }

    /// A `cyamus` command with an isolated environment.
    pub fn cyamus(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cyamus"));
        self.isolate(&mut cmd);
        cmd
    }

    fn isolate(&self, cmd: &mut Command) {
        cmd.env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("SHELL", "/bin/sh")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("EDITOR")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
    }

    /// Runs git in `dir`, panicking on failure, and returns stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = StdCommand::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
    }

    /// A regular repository at `<root>/<name>` on `main` with one commit
    /// containing `tracked.txt` and `.vscode/settings.json`.
    pub fn regular_repo(&self, name: &str) -> PathBuf {
        let repo = self.root.join(name);
        fs::create_dir_all(&repo).unwrap();
        self.git(&repo, &["init", "-q", "-b", "main"]);
        fs::write(repo.join("tracked.txt"), "tracked\n").unwrap();
        fs::create_dir_all(repo.join(".vscode")).unwrap();
        fs::write(repo.join(".vscode/settings.json"), "{}\n").unwrap();
        self.git(&repo, &["add", "."]);
        self.git(&repo, &["commit", "-q", "-m", "init"]);
        repo
    }

    /// A bare repository at `<root>/<name>.git` cloned from a fresh regular
    /// repository, with its `origin` remote removed.
    pub fn bare_repo(&self, name: &str) -> PathBuf {
        let source = self.regular_repo(&format!("{name}-source"));
        let bare = self.root.join(format!("{name}.git"));
        self.git(
            &self.root,
            &[
                "clone",
                "-q",
                "--bare",
                source.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        self.git(&bare, &["remote", "remove", "origin"]);
        bare
    }

    /// Adds a linked worktree for a new branch, like Orca does.
    pub fn add_worktree(&self, repo: &Path, branch: &str) -> PathBuf {
        let dir = self.root.join("worktrees").join(branch.replace('/', "-"));
        self.git(
            repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                dir.to_str().unwrap(),
                "main",
            ],
        );
        dir
    }

    pub fn set_project(&self, repo: &Path, name: &str) {
        self.git(repo, &["config", "--local", "cyamus.project", name]);
    }

    pub fn project_dir(&self, project: &str) -> PathBuf {
        self.config_home.join("cyamus/projects").join(project)
    }

    pub fn cache_dir(&self, project: &str) -> PathBuf {
        self.cache_home.join("cyamus/projects").join(project)
    }

    /// Creates the project config dir with the given manifest.
    pub fn write_manifest(&self, project: &str, content: &str) {
        let dir = self.project_dir(project);
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("cyamus.toml"), content).unwrap();
    }

    pub fn write_asset(&self, project: &str, rel: &str, content: &str) -> PathBuf {
        let path = self.project_dir(project).join("assets").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    pub fn write_bin(&self, project: &str, name: &str, script: &str, executable: bool) -> PathBuf {
        let path = self.project_dir(project).join("bin").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, script).unwrap();
        let mode = if executable { 0o755 } else { 0o644 };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    /// `cyamus workspace <action> <worktree>`.
    pub fn workspace(&self, action: &str, worktree: &Path) -> Command {
        let mut cmd = self.cyamus();
        cmd.args(["workspace", action]).arg(worktree);
        cmd
    }
}

/// Lines of `git status --porcelain`, ignoring nothing.
pub fn status(env: &Env, dir: &Path) -> String {
    env.git(dir, &["status", "--porcelain"])
}

pub fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

pub fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}
