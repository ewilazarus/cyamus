//! Content hashes over sets of worktree files.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::manifest::Fingerprint;

/// Contributed in place of the contents of a missing or unreadable file.
const MISSING_FILE_MARKER: &[u8] = b"NULL";

/// Computes `(name, digest)` for every fingerprint, relative to `worktree`.
///
/// Files are sorted, then each contributes its relative path followed by its
/// contents (or [`MISSING_FILE_MARKER`]) to a single SHA-256 hasher. The hex
/// digest is truncated to the fingerprint's length.
pub fn compute(worktree: &Path, fingerprints: &[Fingerprint]) -> Vec<(String, String)> {
    fingerprints
        .iter()
        .map(|fp| (fp.name.clone(), digest(worktree, fp)))
        .collect()
}

fn digest(worktree: &Path, fp: &Fingerprint) -> String {
    let mut files: Vec<&String> = fp.files.iter().collect();
    files.sort();
    let mut hasher = Sha256::new();
    for rel in files {
        hasher.update(rel.as_bytes());
        match fs::read(worktree.join(rel)) {
            Ok(bytes) => hasher.update(&bytes),
            Err(_) => hasher.update(MISSING_FILE_MARKER),
        }
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    let len = usize::try_from(fp.length)
        .unwrap_or(hex.len())
        .min(hex.len());
    hex.truncate(len);
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(files: &[&str], length: i64) -> Fingerprint {
        Fingerprint {
            name: "deps".to_owned(),
            files: files.iter().map(|f| f.to_string()).collect(),
            length,
        }
    }

    fn sha256_hex(data: &[u8]) -> String {
        let mut hex = String::new();
        for b in Sha256::digest(data) {
            write!(hex, "{b:02x}").unwrap();
        }
        hex
    }

    #[test]
    fn hashes_path_and_contents_in_sorted_order() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), "AAA").unwrap();
        fs::write(dir.path().join("b"), "BBB").unwrap();
        let expected = sha256_hex(b"aAAAbBBB");
        let got = compute(dir.path(), &[fp(&["b", "a"], 64)]);
        assert_eq!(got, [("deps".to_owned(), expected.clone())]);
        let got = compute(dir.path(), &[fp(&["a", "b"], 12)]);
        assert_eq!(got[0].1, expected[..12]);
    }

    #[test]
    fn golden_value() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), "{}\n").unwrap();
        // printf 'package.json{}\n' | shasum -a 256
        let got = compute(dir.path(), &[fp(&["package.json"], 12)]);
        assert_eq!(got[0].1, "5b6314d23e65");
        let got = compute(dir.path(), &[fp(&["package.json"], 64)]);
        assert_eq!(
            got[0].1,
            "5b6314d23e657ad1481742af59264ee6651a2e56dc3381465ea289381df7417d"
        );
    }

    #[test]
    fn missing_file_uses_marker() {
        let dir = tempfile::tempdir().unwrap();
        let got = compute(dir.path(), &[fp(&["Dockerfile"], 64)]);
        assert_eq!(got[0].1, sha256_hex(b"DockerfileNULL"));
    }

    #[test]
    fn changed_contents_change_digest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("lock"), "1").unwrap();
        let before = compute(dir.path(), &[fp(&["lock"], 12)]);
        fs::write(dir.path().join("lock"), "2").unwrap();
        let after = compute(dir.path(), &[fp(&["lock"], 12)]);
        assert_ne!(before, after);
    }
}
