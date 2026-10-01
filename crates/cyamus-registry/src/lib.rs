//! The workspace registry: one TOML record per active workspace under
//! `$XDG_STATE_HOME/cyamus/workspaces/<project>/<label>.toml`.
//!
//! The CLI is the only writer. Writes are serialized by an exclusive lock on
//! `registry.lock` and land atomically (temp file + rename), so the daemon can
//! read the tree at any time without locking.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum length of a DNS label.
pub const MAX_LABEL_LEN: usize = 63;

const RECORD_EXT: &str = "toml";

/// Counter bumped (under the lock) by every change; see [`Registry::change_key`].
const GENERATION_FILE: &str = ".generation";

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("registry I/O failed at {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot encode registry record for {0}")]
    Encode(PathBuf, #[source] toml::ser::Error),
}

/// One registered workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub project: String,
    pub branch: String,
    /// DNS-label form of the branch, used in hostnames.
    pub label: String,
    /// Canonical worktree root.
    pub path: PathBuf,
}

/// Derives the hostname label from a branch slug: at most 63 characters, no
/// trailing hyphen. `None` when nothing usable remains.
pub fn label_from_slug(slug: &str) -> Option<String> {
    let mut label: String = slug.chars().take(MAX_LABEL_LEN).collect();
    label.truncate(label.trim_end_matches('-').len());
    (!label.is_empty()).then_some(label)
}

/// Outcome of [`Registry::register`].
#[derive(Debug, PartialEq, Eq)]
pub enum Registration {
    Registered,
    /// Another live worktree already owns this project and label.
    Conflict {
        existing: PathBuf,
    },
}

/// Handle to a registry directory and its lock file.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
    lock: PathBuf,
}

impl Registry {
    pub fn new(dir: impl Into<PathBuf>, lock: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            lock: lock.into(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn record_path(&self, project: &str, label: &str) -> PathBuf {
        self.dir.join(project).join(format!("{label}.{RECORD_EXT}"))
    }

    /// Records `record` (its path is canonicalized first), unless another
    /// worktree that still exists owns the same project and label.
    pub fn register(&self, record: &Record) -> Result<Registration, RegistryError> {
        let mut record = record.clone();
        record.path = fs::canonicalize(&record.path).map_err(io_at(&record.path))?;
        let _guard = self.lock()?;
        self.gc_locked()?;
        let file = self.record_path(&record.project, &record.label);
        if let Some(existing) = read_record(&file)
            && existing.path != record.path
            && existing.path.exists()
        {
            return Ok(Registration::Conflict {
                existing: existing.path,
            });
        }
        write_atomic(&file, &record)?;
        self.bump()?;
        Ok(Registration::Registered)
    }

    /// Removes the record for `project`/`label` if it belongs to `path`.
    /// Returns whether a record was removed.
    pub fn unregister(
        &self,
        project: &str,
        label: &str,
        path: &Path,
    ) -> Result<bool, RegistryError> {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let _guard = self.lock()?;
        let file = self.record_path(project, label);
        let removed = match read_record(&file) {
            Some(existing) if existing.path == path => {
                remove_file(&file)?;
                self.bump()?;
                true
            }
            _ => false,
        };
        self.gc_locked()?;
        Ok(removed)
    }

    /// Deletes records whose worktree no longer exists and empty project
    /// directories. Returns the removed records.
    pub fn gc(&self) -> Result<Vec<Record>, RegistryError> {
        let _guard = self.lock()?;
        self.gc_locked()
    }

    fn gc_locked(&self) -> Result<Vec<Record>, RegistryError> {
        let mut removed = Vec::new();
        for project_dir in subdirs(&self.dir)? {
            for file in record_files(&project_dir)? {
                if let Some(record) = read_record(&file)
                    && !record.path.exists()
                {
                    remove_file(&file)?;
                    removed.push(record);
                }
            }
            // Only succeeds when empty, which is exactly what we want.
            let _ = fs::remove_dir(&project_dir);
        }
        if !removed.is_empty() {
            self.bump()?;
        }
        Ok(removed)
    }

    /// Reads every well-formed record. Unreadable or malformed files are
    /// skipped; they are never produced by [`Registry::register`].
    pub fn read_all(&self) -> Result<Vec<Record>, RegistryError> {
        let mut records = Vec::new();
        for project_dir in subdirs(&self.dir)? {
            for file in record_files(&project_dir)? {
                if let Some(record) = read_record(&file) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| (&a.project, &a.label).cmp(&(&b.project, &b.label)));
        Ok(records)
    }

    /// A cheap value that changes whenever a record is added, replaced or
    /// removed: a generation counter every write bumps under the lock, plus
    /// every record file's name and inode. The counter is what makes it
    /// reliable: a filesystem may hand a replaced record the inode its
    /// predecessor just freed (ext4 does), leaving names and inodes equal.
    pub fn change_key(&self) -> ChangeKey {
        use std::os::unix::fs::MetadataExt;
        let generation = read_generation(&self.dir);
        let mut entries = Vec::new();
        for dir in subdirs(&self.dir).unwrap_or_default() {
            for file in record_files(&dir).unwrap_or_default() {
                let ino = fs::metadata(&file).map(|m| m.ino()).unwrap_or(0);
                entries.push((file.into_os_string(), ino));
            }
        }
        ChangeKey(generation, entries)
    }

    /// Increments the generation counter. Callers hold the lock.
    fn bump(&self) -> Result<(), RegistryError> {
        fs::create_dir_all(&self.dir).map_err(io_at(&self.dir))?;
        let next = read_generation(&self.dir).wrapping_add(1);
        let path = self.dir.join(GENERATION_FILE);
        let tmp = self
            .dir
            .join(format!("{GENERATION_FILE}.{}.tmp", std::process::id()));
        fs::write(&tmp, format!("{next}\n"))
            .and_then(|()| fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = fs::remove_file(&tmp);
                io_at(&path)(e)
            })
    }

    fn lock(&self) -> Result<File, RegistryError> {
        if let Some(parent) = self.lock.parent() {
            fs::create_dir_all(parent).map_err(io_at(parent))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&self.lock)
            .map_err(io_at(&self.lock))?;
        file.lock().map_err(io_at(&self.lock))?;
        // Released when the returned handle is dropped.
        Ok(file)
    }
}

/// See [`Registry::change_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeKey(u64, Vec<(OsString, u64)>);

fn read_generation(dir: &Path) -> u64 {
    fs::read_to_string(dir.join(GENERATION_FILE))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> RegistryError + '_ {
    move |source| RegistryError::Io {
        path: path.to_owned(),
        source,
    }
}

fn read_record(file: &Path) -> Option<Record> {
    let text = fs::read_to_string(file).ok()?;
    toml::from_str(&text).ok()
}

fn remove_file(file: &Path) -> Result<(), RegistryError> {
    match fs::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_at(file)(e)),
        _ => Ok(()),
    }
}

fn write_atomic(file: &Path, record: &Record) -> Result<(), RegistryError> {
    let dir = file
        .parent()
        .expect("record files live in a project directory");
    fs::create_dir_all(dir).map_err(io_at(dir))?;
    let text = toml::to_string(record).map_err(|e| RegistryError::Encode(file.to_owned(), e))?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        file.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut out = File::create(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        fs::rename(&tmp, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(io_at(file))
}

fn subdirs(dir: &Path) -> Result<Vec<PathBuf>, RegistryError> {
    list(dir, |p| p.is_dir())
}

fn record_files(dir: &Path) -> Result<Vec<PathBuf>, RegistryError> {
    list(dir, |p| {
        p.extension().is_some_and(|e| e == RECORD_EXT)
            && !p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    })
}

fn list(dir: &Path, keep: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>, RegistryError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_at(dir)(e)),
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| keep(p))
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        _tmp: TempDir,
        root: PathBuf,
        registry: Registry,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let registry = Registry::new(
            root.join("state/workspaces"),
            root.join("state/registry.lock"),
        );
        Fixture {
            _tmp: tmp,
            root,
            registry,
        }
    }

    impl Fixture {
        fn worktree(&self, name: &str) -> PathBuf {
            let p = self.root.join(name);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn record(&self, label: &str, path: &Path) -> Record {
            Record {
                project: "myproj".into(),
                branch: label.into(),
                label: label.into(),
                path: path.to_owned(),
            }
        }
    }

    #[test]
    fn labels() {
        assert_eq!(
            label_from_slug("feature-my-thing").as_deref(),
            Some("feature-my-thing")
        );
        let long = format!("{}-{}", "a".repeat(62), "b".repeat(17));
        let label = label_from_slug(&long).unwrap();
        assert_eq!(label, "a".repeat(62));
        assert!(label.len() <= MAX_LABEL_LEN);
        assert_eq!(label_from_slug(&"x".repeat(80)).unwrap().len(), 63);
        assert_eq!(label_from_slug(""), None);
    }

    #[test]
    fn register_writes_canonical_record() {
        let f = fixture();
        let wt = f.worktree("feat");
        let link = f.root.join("feat-link");
        std::os::unix::fs::symlink(&wt, &link).unwrap();
        let outcome = f.registry.register(&f.record("feat-x", &link)).unwrap();
        assert_eq!(outcome, Registration::Registered);
        let file = f.root.join("state/workspaces/myproj/feat-x.toml");
        let record: Record = toml::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(record.path, wt);
        assert_eq!(f.registry.read_all().unwrap(), vec![record]);
    }

    #[test]
    fn re_registration_is_idempotent() {
        let f = fixture();
        let wt = f.worktree("feat");
        let rec = f.record("feat-x", &wt);
        assert_eq!(f.registry.register(&rec).unwrap(), Registration::Registered);
        assert_eq!(f.registry.register(&rec).unwrap(), Registration::Registered);
        assert_eq!(f.registry.read_all().unwrap().len(), 1);
    }

    #[test]
    fn conflict_with_live_worktree() {
        let f = fixture();
        let a = f.worktree("a");
        let b = f.worktree("b");
        f.registry.register(&f.record("feat-x", &a)).unwrap();
        let outcome = f.registry.register(&f.record("feat-x", &b)).unwrap();
        assert_eq!(
            outcome,
            Registration::Conflict {
                existing: a.clone()
            }
        );
        assert_eq!(f.registry.read_all().unwrap()[0].path, a);
    }

    #[test]
    fn stale_record_is_replaced() {
        let f = fixture();
        let a = f.worktree("a");
        let b = f.worktree("b");
        f.registry.register(&f.record("feat-x", &a)).unwrap();
        fs::remove_dir(&a).unwrap();
        let outcome = f.registry.register(&f.record("feat-x", &b)).unwrap();
        assert_eq!(outcome, Registration::Registered);
        assert_eq!(f.registry.read_all().unwrap()[0].path, b);
    }

    #[test]
    fn unregister_only_own_record() {
        let f = fixture();
        let a = f.worktree("a");
        let b = f.worktree("b");
        f.registry.register(&f.record("feat-x", &a)).unwrap();
        assert!(!f.registry.unregister("myproj", "feat-x", &b).unwrap());
        assert_eq!(f.registry.read_all().unwrap().len(), 1);
        assert!(f.registry.unregister("myproj", "feat-x", &a).unwrap());
        assert!(f.registry.read_all().unwrap().is_empty());
        // Missing record is fine, and the empty project dir is gone.
        assert!(!f.registry.unregister("myproj", "feat-x", &a).unwrap());
        assert!(!f.root.join("state/workspaces/myproj").exists());
    }

    #[test]
    fn gc_removes_records_of_deleted_worktrees() {
        let f = fixture();
        let a = f.worktree("a");
        let b = f.worktree("b");
        f.registry.register(&f.record("one", &a)).unwrap();
        f.registry.register(&f.record("two", &b)).unwrap();
        fs::remove_dir(&a).unwrap();
        let removed = f.registry.gc().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].label, "one");
        assert_eq!(f.registry.read_all().unwrap().len(), 1);
    }

    #[test]
    fn concurrent_registrations_are_all_kept() {
        let f = fixture();
        let worktrees: Vec<PathBuf> = (0..16).map(|i| f.worktree(&format!("wt{i}"))).collect();
        std::thread::scope(|s| {
            for (i, wt) in worktrees.iter().enumerate() {
                let registry = f.registry.clone();
                let rec = f.record(&format!("b{i}"), wt);
                s.spawn(move || registry.register(&rec).unwrap());
            }
        });
        assert_eq!(f.registry.read_all().unwrap().len(), 16);
    }

    #[test]
    fn change_key_tracks_register_and_unregister() {
        let f = fixture();
        let wt = f.worktree("a");
        let k0 = f.registry.change_key();
        f.registry.register(&f.record("feat-x", &wt)).unwrap();
        let k1 = f.registry.change_key();
        assert_ne!(k0, k1);
        assert_eq!(k1, f.registry.change_key());
        let other = f.worktree("b");
        f.registry.register(&f.record("feat-y", &other)).unwrap();
        let k2 = f.registry.change_key();
        assert_ne!(k1, k2);
        // Replacing a stale record with the same label is a change too.
        fs::remove_dir(&other).unwrap();
        let again = f.worktree("c");
        f.registry.register(&f.record("feat-y", &again)).unwrap();
        let k3 = f.registry.change_key();
        assert_ne!(k2, k3);
        f.registry.unregister("myproj", "feat-x", &wt).unwrap();
        assert_ne!(k3, f.registry.change_key());
    }
}
