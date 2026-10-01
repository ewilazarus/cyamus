//! Keeps README.md honest: its example manifest parses, and its documented
//! paths match what cyamus uses.

mod common;

use std::path::Path;

use cyamus_core::manifest::Manifest;

const README: &str = include_str!("../../../README.md");

fn toml_blocks() -> Vec<&'static str> {
    README
        .split("```toml\n")
        .skip(1)
        .map(|rest| rest.split("```").next().unwrap())
        .collect()
}

#[test]
fn example_manifest_parses() {
    let blocks = toml_blocks();
    assert!(!blocks.is_empty());
    for block in blocks {
        let manifest = Manifest::parse(block, Path::new("README.md")).unwrap();
        assert!(!manifest.links.is_empty() && !manifest.fingerprints.is_empty());
    }
}

#[test]
fn documented_paths_match() {
    let env = common::Env::new();
    for documented in [
        "$XDG_CONFIG_HOME/cyamus/projects/<project>/",
        "$XDG_CACHE_HOME/cyamus/projects/<project>/",
        "~/.config/cyamus/projects/<project>/",
    ] {
        assert!(
            README.contains(documented),
            "README no longer documents {documented}"
        );
    }
    assert!(env.project_dir("p").ends_with("cyamus/projects/p"));
    assert!(env.cache_dir("p").ends_with("cyamus/projects/p"));
    for entry in ["cyamus.toml", "assets/", "bin/"] {
        assert!(README.contains(entry));
    }
}
