//! The cyamus-managed block in the repository's shared `info/exclude`.
//!
//! The exclude file lives in the git common dir and is shared by every
//! worktree, and Orca may run several setups in parallel. Updates therefore
//! happen under an exclusive lock and are written atomically.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

pub const BEGIN_MARKER: &str = "# >>> cyamus managed >>>";
pub const END_MARKER: &str = "# <<< cyamus managed <<<";

const LOCK_FILE: &str = "cyamus.lock";

/// Replaces the cyamus block in `<common_dir>/info/exclude` with `targets`
/// (paths relative to the worktree root), preserving every other line. An
/// empty `targets` removes the block.
pub fn sync(common_dir: &Path, targets: &[String]) -> io::Result<()> {
    let info = common_dir.join("info");
    fs::create_dir_all(&info)?;

    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(common_dir.join(LOCK_FILE))?;
    lock.lock()?;

    let path = info.join("exclude");
    let current = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let updated = rewrite(&current, targets);
    if updated != current {
        let tmp = info.join(format!("exclude.cyamus-{}.tmp", std::process::id()));
        fs::write(&tmp, &updated)?;
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }

    drop::<File>(lock);
    Ok(())
}

/// Returns `content` with its cyamus block replaced by one listing `targets`.
pub fn rewrite(content: &str, targets: &[String]) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_block = false;
    for line in content.lines() {
        match line {
            BEGIN_MARKER => in_block = true,
            END_MARKER if in_block => in_block = false,
            _ if !in_block => lines.push(line),
            _ => {}
        }
    }
    let mut out = String::new();
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    if !targets.is_empty() {
        out.push_str(BEGIN_MARKER);
        out.push('\n');
        for target in targets {
            out.push_str(&anchor(target));
            out.push('\n');
        }
        out.push_str(END_MARKER);
        out.push('\n');
    }
    out
}

/// Anchors a worktree-relative path at the worktree root (`a/b` -> `/a/b`).
fn anchor(target: &str) -> String {
    let trimmed = target.trim_start_matches("./").trim_start_matches('/');
    let trimmed = trimmed.trim_end_matches('/');
    format!("/{trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn appends_block_after_user_lines() {
        let out = rewrite("# user\n*.log\n", &t(&[".env.local", "./config/x.yaml"]));
        assert_eq!(
            out,
            "# user\n*.log\n# >>> cyamus managed >>>\n/.env.local\n/config/x.yaml\n# <<< cyamus managed <<<\n"
        );
    }

    #[test]
    fn replaces_existing_block_and_is_idempotent() {
        let first = rewrite("a\n", &t(&["x"]));
        let second = rewrite(&first, &t(&["y"]));
        assert_eq!(
            second,
            "a\n# >>> cyamus managed >>>\n/y\n# <<< cyamus managed <<<\n"
        );
        assert_eq!(rewrite(&second, &t(&["y"])), second);
    }

    #[test]
    fn keeps_user_lines_after_block() {
        let content = "a\n# >>> cyamus managed >>>\n/x\n# <<< cyamus managed <<<\nb\n";
        assert_eq!(
            rewrite(content, &t(&["z"])),
            "a\nb\n# >>> cyamus managed >>>\n/z\n# <<< cyamus managed <<<\n"
        );
    }

    #[test]
    fn empty_targets_remove_block() {
        let content = "a\n# >>> cyamus managed >>>\n/x\n# <<< cyamus managed <<<\n";
        assert_eq!(rewrite(content, &[]), "a\n");
    }

    #[test]
    fn adds_trailing_newline_to_user_content() {
        assert_eq!(
            rewrite("a", &t(&["x"])),
            "a\n# >>> cyamus managed >>>\n/x\n# <<< cyamus managed <<<\n"
        );
    }
}
