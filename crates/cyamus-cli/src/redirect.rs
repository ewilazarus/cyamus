//! `cyamus daemon install/uninstall`: the port-80 relay as a system service,
//! so URLs need no port.
//!
//! The daemon never listens on 80. A root-owned copy of the binary runs
//! `daemon relay`, which listens on loopback port 80 only and forwards bytes
//! to the daemon port: from launchd on macOS (binds as root, serves as
//! `nobody`) and from systemd on Linux (as an ephemeral `DynamicUser` with
//! only `CAP_NET_BIND_SERVICE`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Root-owned copy of the binary the service runs. Never the user's own
/// binary: anything running as the user could replace that and get root.
pub const RELAY_BIN: &str = "/usr/local/libexec/cyamus-relay";
pub const LAUNCHD_LABEL: &str = "dev.cyamus.relay";
pub const LAUNCHD_PLIST: &str = "/Library/LaunchDaemons/dev.cyamus.relay.plist";
pub const RELAY_LOG: &str = "/var/log/cyamus-relay.log";
pub const SYSTEMD_UNIT: &str = "cyamus-relay.service";
pub const SYSTEMD_UNIT_PATH: &str = "/etc/systemd/system/cyamus-relay.service";

/// Left behind by the abandoned pf-based redirect; removed if present.
const LEGACY_LABEL: &str = "dev.cyamus.redirect";
const LEGACY_PLIST: &str = "/Library/LaunchDaemons/dev.cyamus.redirect.plist";
const LEGACY_PF_RULES: &str = "/etc/pf.anchors/cyamus";
const LEGACY_PF_ANCHOR: &str = "com.apple/cyamus";

/// The privileged changes for one platform.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Text files to place (root-owned, mode 644).
    pub files: Vec<(PathBuf, String)>,
    /// Commands run before the files are placed (binary copy, cleanup).
    pub prepare: Vec<Step>,
    /// Commands run after the files are in place.
    pub install: Vec<Step>,
    /// Commands that stop the service (before files are removed).
    pub uninstall: Vec<Step>,
    /// Everything uninstall deletes.
    pub remove: Vec<PathBuf>,
    /// Commands run after the files are removed.
    pub cleanup: Vec<Step>,
    /// Tools that must exist before anything is changed.
    pub requires: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub argv: Vec<String>,
    /// Failure is expected in some states (e.g. nothing to unload).
    pub may_fail: bool,
}

fn step(argv: &[&str]) -> Step {
    Step {
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        may_fail: false,
    }
}

fn tolerant(argv: &[&str]) -> Step {
    Step {
        may_fail: true,
        ..step(argv)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
}

impl Os {
    pub fn current() -> Result<Self, String> {
        match std::env::consts::OS {
            "macos" => Ok(Self::MacOs),
            "linux" => Ok(Self::Linux),
            other => Err(format!("the port-80 relay is not supported on {other}")),
        }
    }
}

/// What the plan needs to know about this machine.
pub struct Inputs<'a> {
    pub port: u16,
    /// The running `cyamus` binary, copied to [`RELAY_BIN`].
    pub exe: &'a Path,
    /// `nobody`'s uid (macOS only).
    pub nobody: u32,
}

pub fn plan(os: Os, inputs: &Inputs<'_>) -> Plan {
    match os {
        Os::MacOs => macos(inputs),
        Os::Linux => linux(inputs),
    }
}

fn copy_binary(exe: &Path) -> Vec<Step> {
    vec![
        step(&["mkdir", "-p", "/usr/local/libexec"]),
        step(&["install", "-m", "755", &exe.to_string_lossy(), RELAY_BIN]),
    ]
}

fn macos_legacy_cleanup() -> Vec<Step> {
    vec![
        tolerant(&["launchctl", "bootout", &format!("system/{LEGACY_LABEL}")]),
        tolerant(&["/sbin/pfctl", "-a", LEGACY_PF_ANCHOR, "-F", "all"]),
        step(&["rm", "-f", LEGACY_PLIST, LEGACY_PF_RULES]),
    ]
}

fn macos(inputs: &Inputs<'_>) -> Plan {
    let uid = inputs.nobody;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Managed by `cyamus daemon install`: loopback port 80 -> cyamus daemon. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{RELAY_BIN}</string>
    <string>daemon</string>
    <string>relay</string>
    <string>--to</string>
    <string>{port}</string>
    <string>--user</string>
    <string>{uid}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>{RELAY_LOG}</string>
</dict>
</plist>
"#,
        port = inputs.port
    );
    let label = format!("system/{LAUNCHD_LABEL}");
    let mut prepare = macos_legacy_cleanup();
    prepare.extend(copy_binary(inputs.exe));
    let mut uninstall = vec![tolerant(&["launchctl", "bootout", &label])];
    uninstall.extend(macos_legacy_cleanup());
    Plan {
        files: vec![(LAUNCHD_PLIST.into(), plist)],
        prepare,
        install: vec![
            // Replaces a relay from an earlier install.
            tolerant(&["launchctl", "bootout", &label]),
            step(&["launchctl", "bootstrap", "system", LAUNCHD_PLIST]),
        ],
        uninstall,
        remove: vec![LAUNCHD_PLIST.into(), RELAY_BIN.into()],
        cleanup: vec![],
        requires: vec!["launchctl"],
    }
}

fn linux(inputs: &Inputs<'_>) -> Plan {
    let unit = format!(
        "# Managed by `cyamus daemon install`: loopback port 80 -> cyamus daemon.\n\
         [Unit]\n\
         Description=cyamus: forward loopback port 80 to the cyamus daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={RELAY_BIN} daemon relay --to {port}\n\
         DynamicUser=yes\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\
         CapabilityBoundingSet=CAP_NET_BIND_SERVICE\n\
         NoNewPrivileges=yes\n\
         Restart=always\n\
         RestartSec=1\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        port = inputs.port
    );
    Plan {
        files: vec![(SYSTEMD_UNIT_PATH.into(), unit)],
        prepare: copy_binary(inputs.exe),
        install: vec![
            step(&["systemctl", "daemon-reload"]),
            step(&["systemctl", "enable", SYSTEMD_UNIT]),
            step(&["systemctl", "restart", SYSTEMD_UNIT]),
        ],
        uninstall: vec![tolerant(&["systemctl", "disable", "--now", SYSTEMD_UNIT])],
        remove: vec![SYSTEMD_UNIT_PATH.into(), RELAY_BIN.into()],
        cleanup: vec![tolerant(&["systemctl", "daemon-reload"])],
        requires: vec!["systemctl"],
    }
}

/// `nobody`'s uid, via `id` (no user-database code needed).
pub fn nobody_uid() -> Result<u32, String> {
    let out = Command::new("id")
        .args(["-u", "nobody"])
        .output()
        .map_err(|e| format!("cannot run id: {e}"))?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| "cannot look up the nobody user".to_owned())
}

/// Finds `tool` on `PATH` (or checks an absolute path).
pub fn which(tool: &str) -> Option<PathBuf> {
    let path = Path::new(tool);
    if path.is_absolute() {
        return path.exists().then(|| path.to_owned());
    }
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&dirs)
        .chain(["/usr/sbin", "/sbin", "/bin", "/usr/bin"].map(PathBuf::from))
        .map(|d| d.join(tool))
        .find(|p| p.is_file())
}

/// Runs plans with `sudo` (unless already root), echoing each command.
pub struct Executor {
    pub dry_run: bool,
    pub sudo: bool,
}

impl Executor {
    pub fn new(dry_run: bool) -> Self {
        Self {
            dry_run,
            sudo: !is_root(),
        }
    }

    fn command_line(&self, argv: &[String]) -> Vec<String> {
        let mut line = Vec::new();
        if self.sudo {
            line.push("sudo".to_owned());
        }
        line.extend(argv.iter().cloned());
        line
    }

    pub fn run(&self, step: &Step) -> Result<(), String> {
        let line = self.command_line(&step.argv);
        println!("  $ {}", line.join(" "));
        if self.dry_run {
            return Ok(());
        }
        let status = Command::new(&line[0])
            .args(&line[1..])
            .status()
            .map_err(|e| format!("cannot run {}: {e}", line[0]))?;
        if status.success() || step.may_fail {
            Ok(())
        } else {
            Err(format!("`{}` failed ({status})", line.join(" ")))
        }
    }

    /// Places `content` at `dest` (root-owned, 644) via a staged temp file.
    pub fn place(&self, dest: &Path, content: &str, staging: &Path) -> Result<(), String> {
        if self.dry_run {
            println!("  would write {}:", dest.display());
            for line in content.lines() {
                println!("    | {line}");
            }
        }
        let parent = dest.parent().expect("absolute destination");
        self.run(&step(&["mkdir", "-p", &parent.to_string_lossy()]))?;
        let staged = staging.join(dest.file_name().expect("file destination"));
        if !self.dry_run {
            fs::write(&staged, content)
                .map_err(|e| format!("cannot stage {}: {e}", staged.display()))?;
        }
        self.run(&step(&[
            "install",
            "-m",
            "644",
            &staged.to_string_lossy(),
            &dest.to_string_lossy(),
        ]))
    }

    pub fn remove(&self, paths: &[PathBuf]) -> Result<(), String> {
        let mut argv = vec!["rm".to_owned(), "-f".to_owned()];
        argv.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
        self.run(&Step {
            argv,
            may_fail: false,
        })
    }
}

pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs<'static> {
        Inputs {
            port: 1355,
            exe: Path::new("/home/me/.local/bin/cyamus"),
            nobody: 4_294_967_294,
        }
    }

    fn argvs(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|s| s.argv.join(" ")).collect()
    }

    #[test]
    fn macos_plan() {
        let p = plan(Os::MacOs, &inputs());
        let (path, plist) = &p.files[0];
        assert_eq!(path, Path::new(LAUNCHD_PLIST));
        for expected in [
            "<string>dev.cyamus.relay</string>",
            "<string>/usr/local/libexec/cyamus-relay</string>",
            "<string>--to</string>\n    <string>1355</string>",
            "<string>--user</string>\n    <string>4294967294</string>",
            "<key>KeepAlive</key>\n  <true/>",
        ] {
            assert!(plist.contains(expected), "{expected}\n{plist}");
        }
        let prepare = argvs(&p.prepare);
        assert!(prepare.contains(
            &"install -m 755 /home/me/.local/bin/cyamus /usr/local/libexec/cyamus-relay".to_owned()
        ));
        // The abandoned pf attempt is cleaned up, but pf is never disabled.
        assert!(prepare.contains(&"/sbin/pfctl -a com.apple/cyamus -F all".to_owned()));
        let all: Vec<String> = [&p.prepare, &p.install, &p.uninstall]
            .into_iter()
            .flat_map(|s| argvs(s))
            .collect();
        assert!(
            !all.iter()
                .any(|c| c.contains("pfctl -d") || c.contains("pfctl -e"))
        );
        assert_eq!(
            argvs(&p.install),
            [
                "launchctl bootout system/dev.cyamus.relay",
                "launchctl bootstrap system /Library/LaunchDaemons/dev.cyamus.relay.plist"
            ]
        );
        assert_eq!(
            p.remove,
            [PathBuf::from(LAUNCHD_PLIST), PathBuf::from(RELAY_BIN)]
        );
    }

    #[test]
    fn linux_plan() {
        let p = plan(Os::Linux, &inputs());
        let (path, unit) = &p.files[0];
        assert_eq!(path, Path::new(SYSTEMD_UNIT_PATH));
        for expected in [
            "ExecStart=/usr/local/libexec/cyamus-relay daemon relay --to 1355\n",
            "DynamicUser=yes\n",
            "AmbientCapabilities=CAP_NET_BIND_SERVICE\n",
            "CapabilityBoundingSet=CAP_NET_BIND_SERVICE\n",
            "NoNewPrivileges=yes\n",
            "Restart=always\n",
        ] {
            assert!(unit.contains(expected), "{expected}\n{unit}");
        }
        assert!(!unit.contains("--user"), "systemd drops privileges itself");
        assert_eq!(p.requires, ["systemctl"]);
        assert_eq!(
            argvs(&p.uninstall),
            ["systemctl disable --now cyamus-relay.service"]
        );
    }

    #[test]
    fn sudo_prefix() {
        let as_user = Executor {
            dry_run: true,
            sudo: true,
        };
        assert_eq!(as_user.command_line(&["ls".into()]), ["sudo", "ls"]);
        let as_root = Executor {
            dry_run: true,
            sudo: false,
        };
        assert_eq!(as_root.command_line(&["ls".into()]), ["ls"]);
    }
}
