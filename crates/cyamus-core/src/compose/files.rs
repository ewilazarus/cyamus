//! Compose file selection, mirroring what `docker compose` does on its own
//! (recorded against Compose 5.1.2; see the add-compose-commands design).
//!
//! cyamus always passes its override with `-f`, which turns compose's own
//! discovery off, so it has to be reproduced here.

use std::path::{Path, PathBuf};

/// Base file names, in compose's order of preference.
pub const BASE_NAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// Override file names, in compose's order of preference (one is used).
pub const OVERRIDE_NAMES: [&str; 4] = [
    "compose.override.yml",
    "compose.override.yaml",
    "docker-compose.override.yml",
    "docker-compose.override.yaml",
];

/// The files compose would use when run in `start` with no `-f` and no
/// `COMPOSE_FILE`: the nearest directory upward (to `/`) holding a base file
/// supplies it plus at most one override from the same directory.
pub fn discover(start: &Path) -> Option<Vec<PathBuf>> {
    for dir in start.ancestors() {
        let Some(base) = first_existing(dir, &BASE_NAMES) else {
            continue;
        };
        let mut files = vec![base];
        files.extend(first_existing(dir, &OVERRIDE_NAMES));
        return Some(files);
    }
    None
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Splits a `COMPOSE_FILE` value on `COMPOSE_PATH_SEPARATOR` (default `:`).
pub fn from_compose_file(value: &str, separator: Option<&str>) -> Vec<PathBuf> {
    let separator = separator.filter(|s| !s.is_empty()).unwrap_or(":");
    value
        .split(separator)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(files: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        for f in files {
            fs::write(root.join(f), "services: {}\n").unwrap();
        }
        (tmp, root)
    }

    fn names(files: Option<Vec<PathBuf>>, root: &Path) -> Vec<String> {
        files
            .unwrap_or_default()
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .collect()
    }

    #[test]
    fn each_default_name_alone() {
        for name in BASE_NAMES {
            let (_t, root) = tree(&[name]);
            assert_eq!(names(discover(&root), &root), [name]);
        }
    }

    #[test]
    fn base_preference_matches_compose() {
        let (_t, root) = tree(&BASE_NAMES);
        assert_eq!(names(discover(&root), &root), ["compose.yaml"]);
        // .yml before .yaml for the docker-compose names.
        let (_t, root) = tree(&["docker-compose.yaml", "docker-compose.yml"]);
        assert_eq!(names(discover(&root), &root), ["docker-compose.yml"]);
        let (_t, root) = tree(&["compose.yml", "docker-compose.yaml"]);
        assert_eq!(names(discover(&root), &root), ["compose.yml"]);
    }

    #[test]
    fn one_override_regardless_of_base_name() {
        let (_t, root) = tree(&["docker-compose.yml", "compose.override.yaml"]);
        assert_eq!(
            names(discover(&root), &root),
            ["docker-compose.yml", "compose.override.yaml"]
        );
        let (_t, root) = tree(&["compose.yaml"]);
        let mut all = vec!["compose.yaml"];
        all.extend(OVERRIDE_NAMES);
        let (_t2, root2) = tree(&all);
        assert_eq!(names(discover(&root), &root), ["compose.yaml"]);
        assert_eq!(
            names(discover(&root2), &root2),
            ["compose.yaml", "compose.override.yml"]
        );
    }

    #[test]
    fn upward_search_nearest_wins() {
        let (_t, root) = tree(&["compose.yaml", "compose.override.yaml", "sub/compose.yaml"]);
        assert_eq!(
            names(discover(&root.join("sub/deep")), &root),
            ["sub/compose.yaml"]
        );
        let (_t, root) = tree(&["compose.yaml", "compose.override.yaml"]);
        assert_eq!(
            names(discover(&root.join("sub/deep")), &root),
            ["compose.yaml", "compose.override.yaml"]
        );
    }

    #[test]
    fn override_only_next_to_the_base() {
        let (_t, root) = tree(&["compose.yaml", "sub/compose.override.yaml"]);
        assert_eq!(names(discover(&root.join("sub")), &root), ["compose.yaml"]);
    }

    #[test]
    fn compose_file_variable() {
        assert_eq!(
            from_compose_file("a.yaml:b.yaml", None),
            [PathBuf::from("a.yaml"), PathBuf::from("b.yaml")]
        );
        assert_eq!(
            from_compose_file("a.yaml;b.yaml", Some(";")),
            [PathBuf::from("a.yaml"), PathBuf::from("b.yaml")]
        );
    }
}
