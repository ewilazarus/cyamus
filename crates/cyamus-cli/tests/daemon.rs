//! daemon spec, end to end through the `cyamus` binary. Each test gets its own
//! state directory and port, and stops its daemon when done.

mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use common::{Env, stderr, stdout};

/// A test environment with the daemon switched on, on a private port.
struct Daemon {
    env: Env,
    port: u16,
}

impl Daemon {
    fn new() -> Self {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Self {
            env: Env::new(),
            port,
        }
    }

    fn configure(&self, cmd: &mut Command) {
        cmd.env("CYAMUS_DAEMON", "on")
            .env("CYAMUS_DAEMON_PORT", self.port.to_string())
            // Keep tests independent of whatever Docker the machine has.
            .env("DOCKER_HOST", "unix:///nonexistent/docker.sock");
    }

    fn cyamus(&self) -> Command {
        let mut cmd = self.env.cyamus();
        self.configure(&mut cmd);
        cmd
    }

    /// `cyamus daemon run` in the foreground, as a child process.
    fn spawn_foreground(&self) -> Child {
        self.env
            .std_cyamus()
            .env("CYAMUS_DAEMON", "on")
            .env("CYAMUS_DAEMON_PORT", self.port.to_string())
            .env("DOCKER_HOST", "unix:///nonexistent/docker.sock")
            .args(["daemon", "run"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn status(&self) -> (bool, String) {
        let out = self.cyamus().args(["daemon", "status"]).output().unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    fn wait_until_up(&self) {
        let start = Instant::now();
        while !self.status().0 {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "daemon did not come up"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn pid_file(&self) -> PathBuf {
        self.env.state_home.join("cyamus/daemon.pid")
    }

    fn pid(&self) -> u32 {
        fs::read_to_string(self.pid_file())
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn port_free(&self) -> bool {
        TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)),
            Duration::from_millis(200),
        )
        .is_err()
    }

    /// A configured project with one worktree whose setup hook touches `ran`.
    fn worktree(&self) -> PathBuf {
        let repo = self.env.regular_repo("repo");
        self.env.set_project(&repo, "myproj");
        self.env
            .write_manifest("myproj", "[[hooks.on_setup]]\ncommands = ['touch ran']\n");
        self.env.add_worktree(&repo, "feat")
    }

    fn setup(&self, wt: &Path) -> Command {
        let mut cmd = self.env.workspace("setup", wt);
        self.configure(&mut cmd);
        cmd
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.cyamus().args(["daemon", "stop"]).output();
    }
}

fn alive(pid: u32) -> bool {
    StdCommand::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn foreground_run_single_instance_and_sigterm() {
    let d = Daemon::new();
    let mut child = d.spawn_foreground();
    d.wait_until_up();
    assert_eq!(d.pid(), child.id());

    let (ok, out) = d.status();
    assert!(ok);
    assert!(
        out.contains(&format!("(pid {}) on port {}", child.id(), d.port)),
        "{out}"
    );
    assert!(out.contains("docker: not reachable"), "{out}");
    assert!(out.contains("routes: none"), "{out}");

    let second = d.cyamus().args(["daemon", "run"]).assert().failure();
    assert!(
        stderr(&second).contains(&format!("already running (pid {})", child.id())),
        "{}",
        stderr(&second)
    );

    StdCommand::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
    assert!(!d.pid_file().exists());
    assert!(d.port_free());
    let mut log = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    assert!(
        log.contains("received SIGTERM") && log.contains("stopped"),
        "{log}"
    );
}

#[test]
fn port_in_use_fails_run() {
    let d = Daemon::new();
    let _v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, d.port)).unwrap();
    let _v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, d.port));
    let assert = d.cyamus().args(["daemon", "run"]).assert().failure();
    let err = stderr(&assert);
    assert!(
        err.contains(&format!("port {}", d.port)) && err.contains("CYAMUS_DAEMON_PORT"),
        "{err}"
    );
}

#[test]
fn status_and_stop_when_not_running() {
    let d = Daemon::new();
    let (ok, out) = d.status();
    assert!(!ok);
    assert!(out.contains("not running"), "{out}");
    let stop = d.cyamus().args(["daemon", "stop"]).assert().success();
    assert!(stdout(&stop).contains("not running"));
}

#[test]
fn setup_starts_a_detached_daemon_and_stop_ends_it() {
    let d = Daemon::new();
    let wt = d.worktree();
    let setup = d.setup(&wt).assert().success();
    assert!(!stderr(&setup).contains("warning"), "{}", stderr(&setup));
    // setup has exited; the daemon keeps answering.
    let (ok, out) = d.status();
    assert!(ok, "{out}");
    let pid = d.pid();
    assert!(alive(pid));
    assert!(d.env.state_home.join("cyamus/daemon.log").exists());

    // A second setup reuses it.
    d.setup(&wt).assert().success();
    assert_eq!(d.pid(), pid);

    let stop = d.cyamus().args(["daemon", "stop"]).assert().success();
    assert!(stdout(&stop).contains(&format!("pid {pid}")));
    assert!(!alive(pid));
    assert!(d.port_free());
}

#[test]
fn parallel_setups_start_one_daemon() {
    let d = Daemon::new();
    let repo = d.env.regular_repo("repo");
    d.env.set_project(&repo, "myproj");
    d.env.write_manifest("myproj", "");
    let worktrees: Vec<PathBuf> = (0..3)
        .map(|i| d.env.add_worktree(&repo, &format!("b{i}")))
        .collect();
    // Spawn from one thread, then wait: setups still run concurrently, but
    // no child is forked while another's pipes are being created. (On macOS
    // std marks new pipes close-on-exec only after creating them, so forking
    // from several threads at once can leak one child's pipe into another
    // and from there into the long-lived daemon.)
    let children: Vec<Child> = worktrees
        .iter()
        .map(|wt| {
            let mut cmd: StdCommand = d.env.std_cyamus();
            cmd.args(["workspace", "setup"])
                .arg(wt)
                .env("CYAMUS_DAEMON", "on")
                .env("CYAMUS_DAEMON_PORT", d.port.to_string())
                .env("DOCKER_HOST", "unix:///nonexistent/docker.sock")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            cmd.spawn().unwrap()
        })
        .collect();
    let outputs: Vec<std::process::Output> = children
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .collect();
    for out in &outputs {
        assert!(out.status.success());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!err.contains("warning"), "{err}");
    }
    let (ok, out) = d.status();
    assert!(ok);
    assert!(out.contains(&format!("(pid {})", d.pid())), "{out}");
    // The losers of the race would have logged a second startup.
    let log = fs::read_to_string(d.env.state_home.join("cyamus/daemon.log")).unwrap();
    assert_eq!(log.matches("listening on").count(), 1, "{log}");
}

#[test]
fn daemon_off_starts_nothing() {
    let d = Daemon::new();
    let wt = d.worktree();
    d.setup(&wt).env("CYAMUS_DAEMON", "off").assert().success();
    assert!(!d.status().0);
}

#[test]
fn held_port_warns_but_setup_succeeds() {
    let d = Daemon::new();
    let wt = d.worktree();
    let _v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, d.port)).unwrap();
    let _v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, d.port));
    let assert = d.setup(&wt).assert().success();
    let err = stderr(&assert);
    assert!(
        err.contains("warning: routing daemon is not running"),
        "{err}"
    );
    assert!(err.contains("daemon.log"), "{err}");
    assert!(wt.join("ran").exists(), "hooks still run");
    assert!(d.env.record("myproj", "feat").is_some());
}

#[test]
fn outdated_daemon_is_replaced() {
    let d = Daemon::new();
    let wt = d.worktree();
    // A stand-in "old daemon": a sleeping process, fronted by a fake status
    // endpoint that reports an old version and that process's PID.
    let old = StdCommand::new("sleep").arg("30").spawn().unwrap();
    let old_pid = old.id();
    // Reap it as soon as it dies, as init would for a real detached daemon.
    let reaper = thread::spawn(move || {
        let mut old = old;
        old.wait().unwrap()
    });
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, d.port)).unwrap();
    let fake = thread::spawn(move || {
        // Setup probes twice: before and after taking the spawn lock.
        for _ in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let body = format!(
                r#"{{"pid":{old_pid},"version":"0.0.0","port":0,"docker":{{"reachable":false,"endpoint":"","source":"","error":null}},"routes":[],"unrouted":[]}}"#
            );
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
        // Dropping the listener frees the port for the real daemon.
    });
    let setup = d.setup(&wt).assert().success();
    assert!(!stderr(&setup).contains("warning"), "{}", stderr(&setup));
    fake.join().unwrap();
    let exit = reaper.join().unwrap();
    assert!(!exit.success(), "the old daemon was terminated: {exit}");
    let (ok, out) = d.status();
    assert!(ok);
    assert!(
        out.contains(&format!("cyamus daemon {}", env!("CARGO_PKG_VERSION"))),
        "{out}"
    );
}

// --- port-80 relay ---

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// GET /api/status with `Host: cyamus.localhost` through `addr`; the raw
/// response, or empty when the connection was closed without one.
fn admin_get(addr: SocketAddr) -> String {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    s.write_all(b"GET /api/status HTTP/1.1\r\nHost: cyamus.localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_port(port: u16) {
    let start = Instant::now();
    while TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "port {port} never opened"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn relay_forwards_to_the_daemon() {
    let d = Daemon::new();
    let mut daemon = d.spawn_foreground();
    d.wait_until_up();
    let listen = free_port();
    let _relay = Killed(
        d.env
            .std_cyamus()
            .args([
                "daemon",
                "relay",
                "--listen",
                &listen.to_string(),
                "--to",
                &d.port.to_string(),
            ])
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for_port(listen);
    for ip in [
        std::net::IpAddr::from(Ipv4Addr::LOCALHOST),
        Ipv6Addr::LOCALHOST.into(),
    ] {
        let resp = admin_get(SocketAddr::new(ip, listen));
        assert!(resp.starts_with("HTTP/1.1 200"), "{ip}: {resp}");
        assert!(
            resp.contains(&format!("\"pid\": {}", daemon.id())),
            "{ip}: {resp}"
        );
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[test]
fn relay_serves_inherited_sockets() {
    use std::os::fd::OwnedFd;
    let d = Daemon::new();
    let mut daemon = d.spawn_foreground();
    d.wait_until_up();
    // What the root parent does: bind, then hand the sockets over as
    // stdin (IPv4) and stdout (IPv6).
    let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = v4.local_addr().unwrap().port();
    let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).ok();
    let _relay = Killed(
        d.env
            .std_cyamus()
            .args([
                "daemon",
                "relay",
                "--inherited",
                "--to",
                &d.port.to_string(),
            ])
            .stdin(Stdio::from(OwnedFd::from(v4)))
            .stdout(v6.map_or_else(Stdio::null, |l| Stdio::from(OwnedFd::from(l))))
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let resp = admin_get(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(
        resp.contains(&format!("\"pid\": {}", daemon.id())),
        "{resp}"
    );
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[test]
fn relay_closes_clients_when_daemon_is_down() {
    let d = Daemon::new();
    let listen = free_port();
    let _relay = Killed(
        d.env
            .std_cyamus()
            .args([
                "daemon",
                "relay",
                "--listen",
                &listen.to_string(),
                "--to",
                &free_port().to_string(),
            ])
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for_port(listen);
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, listen));
    assert_eq!(admin_get(addr), "", "closed without a response");
    // Still accepting afterwards.
    assert_eq!(admin_get(addr), "");
}

#[test]
fn install_dry_run_prints_plan_and_changes_nothing() {
    let d = Daemon::new();
    let assert = d
        .cyamus()
        .args(["daemon", "install", "--dry-run"])
        .assert()
        .success();
    let out = stdout(&assert);
    assert!(out.contains("dry run: nothing is changed"), "{out}");
    assert!(out.contains("would write"), "{out}");
    let exe = assert_cmd::cargo::cargo_bin!("cyamus");
    assert!(
        out.contains(&format!(
            "install -m 755 {} /usr/local/libexec/cyamus-relay",
            exe.display()
        )),
        "{out}"
    );
    let port = d.port;
    if cfg!(target_os = "macos") {
        assert!(
            out.contains("would write /Library/LaunchDaemons/dev.cyamus.relay.plist"),
            "{out}"
        );
        assert!(out.contains(&format!("<string>{port}</string>")), "{out}");
        assert!(
            out.contains(
                "$ sudo launchctl bootstrap system /Library/LaunchDaemons/dev.cyamus.relay.plist"
            ),
            "{out}"
        );
    } else {
        assert!(
            out.contains(&format!(
                "ExecStart=/usr/local/libexec/cyamus-relay daemon relay --to {port}"
            )),
            "{out}"
        );
    }
    // Nothing was started or staged.
    assert!(!d.status().0);
    let staged: Vec<_> = fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("cyamus-install-")
        })
        .collect();
    assert!(staged.is_empty(), "staging left behind: {staged:?}");
}

#[test]
fn uninstall_dry_run_prints_plan() {
    let d = Daemon::new();
    let assert = d
        .cyamus()
        .args(["daemon", "uninstall", "--dry-run"])
        .assert()
        .success();
    let out = stdout(&assert);
    assert!(out.contains("dry run: nothing is changed"), "{out}");
    if cfg!(target_os = "macos") {
        assert!(
            out.contains("$ sudo launchctl bootout system/dev.cyamus.relay"),
            "{out}"
        );
        assert!(
            out.contains("$ sudo rm -f /Library/LaunchDaemons/dev.cyamus.relay.plist /usr/local/libexec/cyamus-relay"),
            "{out}"
        );
        assert!(!out.contains("pfctl -d"), "never disables pf: {out}");
    } else {
        assert!(
            out.contains("systemctl disable --now cyamus-relay.service"),
            "{out}"
        );
    }
}

#[test]
fn status_reports_missing_redirect() {
    let d = Daemon::new();
    let wt = d.worktree();
    d.setup(&wt).assert().success();
    let (ok, out) = d.status();
    assert!(ok, "{out}");
    // Port 80 on this machine may reach some *other* daemon; ours only counts
    // when the PID matches, which a test daemon's never does.
    assert!(
        out.contains("port 80: not redirected; run `cyamus daemon install`"),
        "{out}"
    );
}
