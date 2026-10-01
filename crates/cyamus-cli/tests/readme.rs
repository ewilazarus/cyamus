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
fn example_manifests_parse() {
    let blocks = toml_blocks();
    assert!(blocks.len() >= 2, "reference manifest and Docker example");
    let manifests: Vec<Manifest> = blocks
        .iter()
        .map(|block| Manifest::parse(block, Path::new("README.md")).unwrap())
        .collect();
    // The first block is the reference manifest and shows every feature.
    let reference = &manifests[0];
    assert!(!reference.links.is_empty() && !reference.fingerprints.is_empty());
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

#[test]
fn documented_daemon_settings_match() {
    use cyamus_core::daemon::{DEFAULT_PORT, PORT_VAR};
    use cyamus_daemon::host::ADMIN_HOST;
    use cyamus_daemon::routes::{
        LABEL_ENABLE, LABEL_PORT, LABEL_PROJECT, LABEL_SERVICE, LABEL_WORKSPACE,
    };
    for documented in [
        format!(".localhost:{DEFAULT_PORT}/"),
        format!("http://{ADMIN_HOST}/"),
        format!("http://{ADMIN_HOST}:{DEFAULT_PORT}/"),
        "cyamus daemon install".to_owned(),
        "cyamus daemon uninstall".to_owned(),
        "`CYAMUS_URL_SUFFIX`".to_owned(),
        format!("`{PORT_VAR}`"),
        "`CYAMUS_DAEMON=off`".to_owned(),
        "$XDG_STATE_HOME/cyamus/workspaces/".to_owned(),
        "$XDG_STATE_HOME/cyamus/daemon.log".to_owned(),
    ] {
        assert!(
            README.contains(&documented),
            "README no longer documents {documented}"
        );
    }
    // What `cyamus daemon install` creates (crates/cyamus-cli/src/redirect.rs).
    for path in [
        "/usr/local/libexec/cyamus-relay",
        "/Library/LaunchDaemons/dev.cyamus.relay.plist",
        "/etc/systemd/system/cyamus-relay.service",
        "/var/log/cyamus-relay.log",
    ] {
        assert!(README.contains(path), "README no longer documents {path}");
    }
    for label in [
        LABEL_ENABLE,
        LABEL_SERVICE,
        LABEL_PORT,
        LABEL_PROJECT,
        LABEL_WORKSPACE,
    ] {
        assert!(
            README.contains(&format!("`{label}`")),
            "README label table lacks {label}"
        );
    }
}
