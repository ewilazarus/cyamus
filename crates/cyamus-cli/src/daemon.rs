//! `cyamus daemon ...` and the [`DaemonControl`] that `workspace setup` uses
//! to start the daemon on demand.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use cyamus_core::daemon::DaemonControl;
use cyamus_core::paths::Dirs;
use cyamus_daemon::{Status, VERSION};
use cyamus_registry::Registry;
use rustix::process::{Pid, Signal, kill_process, test_kill_process};

use crate::redirect::{self, Executor, Os};

/// `CYAMUS_DAEMON=off` keeps setup from starting the daemon.
const SWITCH_VAR: &str = "CYAMUS_DAEMON";
const READY_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

pub fn run(dirs: &Dirs, port: u16, background: bool) -> anyhow::Result<ExitCode> {
    if background {
        // Leave the caller's session so closing its terminal can't signal us.
        // Fails only if we already lead a process group, which spawn avoids.
        let _ = rustix::process::setsid();
    }
    cyamus_daemon::run(dirs, port, |k| std::env::var_os(k))?;
    Ok(ExitCode::SUCCESS)
}

pub fn status(dirs: &Dirs, port: u16) -> anyhow::Result<ExitCode> {
    let _ = Registry::new(dirs.registry_dir(), dirs.registry_lock()).gc();
    let Some(status) = probe(port) else {
        match running_pid(dirs) {
            Some(pid) => outln!(
                "cyamus daemon (pid {pid}) is running but not answering on port {port}; is CYAMUS_DAEMON_PORT different from when it started?"
            ),
            None => outln!("cyamus daemon is not running"),
        }
        return Ok(ExitCode::FAILURE);
    };
    out!("{}", render(&status, redirect_active(&status)));
    Ok(ExitCode::SUCCESS)
}

/// Port the optional `cyamus daemon install` redirect listens on.
pub const REDIRECT_PORT: u16 = 80;

/// Whether loopback port 80 reaches the daemon described by `status`.
pub fn redirect_active(status: &Status) -> bool {
    redirect_active_via(REDIRECT_PORT, status)
}

fn redirect_active_via(redirect_port: u16, status: &Status) -> bool {
    probe(redirect_port).is_some_and(|s| s.pid == status.pid)
}

fn url_suffix(status: &Status, redirected: bool) -> String {
    if redirected {
        String::new()
    } else {
        format!(":{}", status.port)
    }
}

fn render(status: &Status, redirected: bool) -> String {
    let mut out = format!(
        "cyamus daemon {} (pid {}) on port {}\n",
        status.version, status.pid, status.port
    );
    let docker = &status.docker;
    if docker.reachable {
        out.push_str(&format!(
            "docker: connected to {} ({})\n",
            docker.endpoint, docker.source
        ));
    } else {
        let why = docker.error.as_deref().unwrap_or("connecting");
        out.push_str(&format!(
            "docker: not reachable via {}: {why}\n",
            docker.source
        ));
    }
    if redirected {
        out.push_str("port 80: redirected to the daemon (URLs need no port)\n");
    } else {
        out.push_str(&format!(
            "port 80: not redirected; run `cyamus daemon install` to drop :{} from URLs\n",
            status.port
        ));
    }
    let suffix = url_suffix(status, redirected);
    if status.routes.is_empty() {
        out.push_str("routes: none\n");
    } else {
        out.push_str("routes:\n");
        for route in &status.routes {
            out.push_str(&format!(
                "  {}  ->  {}  ({})\n",
                status.url(route, &suffix),
                route.target,
                route.container
            ));
        }
    }
    if !status.unrouted.is_empty() {
        out.push_str("not routed:\n");
        for u in &status.unrouted {
            let owner = match (&u.project, &u.workspace) {
                (Some(p), Some(w)) => format!(" ({p}/{w})"),
                (Some(p), None) => format!(" ({p}/shared)"),
                _ => String::new(),
            };
            out.push_str(&format!("  {}{owner}: {}\n", u.container, u.reason));
        }
    }
    out
}

pub fn stop(dirs: &Dirs, port: u16) -> anyhow::Result<ExitCode> {
    let pid = probe(port).map(|s| s.pid).or_else(|| running_pid(dirs));
    match pid {
        None => outln!("cyamus daemon is not running"),
        Some(pid) => {
            terminate(pid).map_err(anyhow::Error::msg)?;
            outln!("stopped cyamus daemon (pid {pid})");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `cyamus daemon relay`: forward loopback port 80 (or `listen`) to `to`.
///
/// Started as root with `--user`, it binds, then replaces itself with
/// `sudo -u #<uid>` running the serving relay; the sockets travel as the
/// child's stdin (IPv4) and stdout (IPv6), inetd-style. sudo sets the real,
/// effective and saved ids and the group list through the platform's own
/// user context; std's `CommandExt::uid/gid` does not on macOS, where its
/// `setgroups(0, NULL)` resets the effective gid to 0.
pub fn relay(to: u16, listen: u16, user: Option<u32>, inherited: bool) -> anyhow::Result<ExitCode> {
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::process::CommandExt;

    if inherited {
        ensure_unprivileged()?;
        let inherit = |fd: std::io::Result<OwnedFd>| {
            fd.ok()
                .map(std::net::TcpListener::from)
                // /dev/null (a family that failed to bind) has no address.
                .filter(|l| l.local_addr().is_ok())
        };
        let listeners: Vec<_> = [
            inherit(std::io::stdin().as_fd().try_clone_to_owned()),
            inherit(std::io::stdout().as_fd().try_clone_to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect();
        cyamus_daemon::relay::serve(listeners, to).context("relay failed")?;
        return Ok(ExitCode::SUCCESS);
    }

    let sockets = cyamus_daemon::bind_loopback(listen).map_err(|(v4, v6)| {
        anyhow::anyhow!("cannot listen on port {listen} (127.0.0.1: {v4}; ::1: {v6})")
    })?;
    let Some(uid) = user.filter(|_| redirect::is_root()) else {
        cyamus_daemon::relay::serve(sockets.into_vec(), to).context("relay failed")?;
        return Ok(ExitCode::SUCCESS);
    };
    let pass = |l: Option<std::net::TcpListener>| match l {
        Some(l) => Stdio::from(OwnedFd::from(l)),
        None => Stdio::null(),
    };
    let exe = std::env::current_exe().context("cannot locate the relay binary")?;
    let error = Command::new("/usr/bin/sudo")
        .args(["-n", "-u", &format!("#{uid}"), "--"])
        .arg(exe)
        .args(["daemon", "relay", "--to", &to.to_string(), "--inherited"])
        .stdin(pass(sockets.v4))
        .stdout(pass(sockets.v6))
        .exec();
    // `exec` only returns on failure.
    Err(anyhow::Error::new(error).context("cannot start the unprivileged relay"))
}

/// The serving relay refuses to run with any root identity left: real or
/// effective uid or gid 0, or group 0 in its group list.
fn ensure_unprivileged() -> anyhow::Result<()> {
    use rustix::process::{getegid, geteuid, getgid, getgroups, getuid};
    let groups = getgroups().context("cannot read the group list")?;
    let privileged = getuid().is_root()
        || geteuid().is_root()
        || getgid().is_root()
        || getegid().is_root()
        || groups.iter().any(|g| g.is_root());
    if privileged {
        anyhow::bail!(
            "refusing to serve: still privileged (uid {}/{}, gid {}/{}, groups {:?})",
            getuid().as_raw(),
            geteuid().as_raw(),
            getgid().as_raw(),
            getegid().as_raw(),
            groups.iter().map(|g| g.as_raw()).collect::<Vec<_>>()
        );
    }
    Ok(())
}

/// `cyamus daemon install`: the port-80 relay as a system service.
pub fn install(dirs: &Dirs, port: u16, dry_run: bool) -> anyhow::Result<ExitCode> {
    let os = Os::current().map_err(anyhow::Error::msg)?;
    let exe = std::env::current_exe().context("cannot locate the cyamus binary")?;
    let nobody = match os {
        Os::MacOs => redirect::nobody_uid().map_err(anyhow::Error::msg)?,
        Os::Linux => 0,
    };
    let plan = redirect::plan(
        os,
        &redirect::Inputs {
            port,
            exe: &exe,
            nobody,
        },
    );
    let missing: Vec<&str> = plan
        .requires
        .iter()
        .copied()
        .filter(|tool| redirect::which(tool).is_none())
        .collect();
    if !missing.is_empty() {
        let missing = missing.join(" and ");
        if !dry_run {
            anyhow::bail!(
                "cannot install the port-80 relay: {missing} not found; nothing was changed"
            );
        }
        outln!("note: {missing} not found; a real install would stop here");
    }
    outln!(
        "Installing the port-80 relay to the cyamus daemon on port {port}{}",
        if dry_run {
            " (dry run: nothing is changed)"
        } else if redirect::is_root() {
            ""
        } else {
            " (this uses sudo)"
        }
    );
    let exec = Executor::new(dry_run);
    let staging = std::env::temp_dir().join(format!("cyamus-install-{}", std::process::id()));
    if !dry_run {
        fs::create_dir_all(&staging).context("cannot create a staging directory")?;
    }
    let applied = (|| {
        plan.prepare.iter().try_for_each(|step| exec.run(step))?;
        for (dest, content) in &plan.files {
            exec.place(dest, content, &staging)?;
        }
        plan.install.iter().try_for_each(|step| exec.run(step))
    })();
    let _ = fs::remove_dir_all(&staging);
    applied.map_err(anyhow::Error::msg)?;
    if dry_run {
        return Ok(ExitCode::SUCCESS);
    }

    if redirect::is_root() {
        // A daemon started here would run as root with root's state; leave
        // that to the user's next `workspace setup` or `daemon status`.
        if probe(port).is_none() {
            outln!(
                "Installed. The relay forwards to port {port}; URLs need no port once your daemon runs (`cyamus workspace setup` starts it)."
            );
            return Ok(ExitCode::SUCCESS);
        }
    } else {
        Spawner { dirs, port }
            .ensure_running()
            .map_err(anyhow::Error::msg)?;
    }
    let start = Instant::now();
    while start.elapsed() < READY_TIMEOUT {
        if probe(port).is_some_and(|s| redirect_active(&s)) {
            outln!(
                "Done: http://cyamus.localhost/ reaches the daemon. URLs no longer need :{port}."
            );
            return Ok(ExitCode::SUCCESS);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "the relay is installed, but http://cyamus.localhost/ does not reach the daemon on port {port}; see {}",
        match os {
            Os::MacOs => redirect::RELAY_LOG.to_owned(),
            Os::Linux => format!("journalctl -u {}", redirect::SYSTEMD_UNIT),
        }
    )
}

/// `cyamus daemon uninstall`: remove what `install` set up.
pub fn uninstall(port: u16, dry_run: bool) -> anyhow::Result<ExitCode> {
    let os = Os::current().map_err(anyhow::Error::msg)?;
    let plan = redirect::plan(
        os,
        &redirect::Inputs {
            port,
            exe: std::path::Path::new(redirect::RELAY_BIN),
            nobody: 0,
        },
    );
    outln!(
        "Removing the port-80 relay{}",
        if dry_run {
            " (dry run: nothing is changed)"
        } else if redirect::is_root() {
            ""
        } else {
            " (this uses sudo)"
        }
    );
    let exec = Executor::new(dry_run);
    let removed = (|| {
        plan.uninstall.iter().try_for_each(|step| exec.run(step))?;
        exec.remove(&plan.remove)?;
        plan.cleanup.iter().try_for_each(|step| exec.run(step))
    })();
    removed.map_err(anyhow::Error::msg)?;
    if !dry_run {
        outln!("Done. URLs need :{port} again.");
    }
    Ok(ExitCode::SUCCESS)
}

/// The daemon's PID, if a daemon holds the lock. A PID file left behind by
/// a crashed daemon doesn't count.
fn running_pid(dirs: &Dirs) -> Option<u32> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dirs.daemon_lock())
        .ok()?;
    match lock.try_lock() {
        Err(TryLockError::WouldBlock) => cyamus_daemon::read_pid(dirs),
        _ => None,
    }
}

fn alive(pid: u32) -> bool {
    pid_of(pid).is_some_and(|p| test_kill_process(p).is_ok())
}

fn pid_of(pid: u32) -> Option<Pid> {
    i32::try_from(pid).ok().and_then(Pid::from_raw)
}

/// SIGTERM, then wait for the process to go away.
fn terminate(pid: u32) -> Result<(), String> {
    let target = pid_of(pid).ok_or_else(|| format!("invalid pid {pid}"))?;
    match kill_process(target, Signal::TERM) {
        Ok(()) => {}
        Err(rustix::io::Errno::SRCH) => return Ok(()),
        Err(e) => return Err(format!("cannot signal daemon (pid {pid}): {e}")),
    }
    let start = Instant::now();
    while alive(pid) {
        if start.elapsed() > STOP_TIMEOUT {
            return Err(format!(
                "daemon (pid {pid}) did not exit within {STOP_TIMEOUT:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Asks the daemon on `port` for its status with one blocking HTTP/1.1 GET.
pub fn probe(port: u16) -> Option<Status> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .ok()?;
    stream
        .write_all(
            b"GET /api/status HTTP/1.1\r\nHost: cyamus.localhost\r\nConnection: close\r\n\r\n",
        )
        .ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8(response).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    serde_json::from_str(body).ok()
}

fn daemon_switched_off() -> bool {
    std::env::var_os(SWITCH_VAR).is_some_and(|v| v == "off")
}

/// Starts the daemon for `workspace setup`.
pub struct Spawner<'a> {
    pub dirs: &'a Dirs,
    pub port: u16,
}

impl DaemonControl for Spawner<'_> {
    fn port(&self) -> u16 {
        self.port
    }

    fn url_suffix(&self) -> String {
        // With the daemon switched off cyamus doesn't manage it, so don't
        // probe; `:<port>` URLs work whether or not a redirect exists.
        if daemon_switched_off() {
            return format!(":{}", self.port);
        }
        match probe(self.port) {
            Some(status) if redirect_active(&status) => String::new(),
            _ => format!(":{}", self.port),
        }
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        if daemon_switched_off() {
            return Ok(());
        }
        if probe(self.port).is_some_and(|s| s.version == VERSION) {
            return Ok(());
        }
        // Serialize spawners; the daemon itself holds daemon.lock.
        let _guard = self.spawn_lock()?;
        match probe(self.port) {
            Some(s) if s.version == VERSION => return Ok(()),
            Some(s) => terminate(s.pid)?,
            None => {
                if let Some(pid) = running_pid(self.dirs) {
                    return Err(format!(
                        "a daemon (pid {pid}) is already running but not on port {}; stop it with `cyamus daemon stop` using its port, or align CYAMUS_DAEMON_PORT",
                        self.port
                    ));
                }
            }
        }
        self.spawn()
    }
}

impl Spawner<'_> {
    fn spawn_lock(&self) -> Result<File, String> {
        let dir = self.dirs.state_dir();
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let path = dir.join("daemon.spawn.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        file.lock()
            .map_err(|e| format!("cannot lock {}: {e}", path.display()))?;
        Ok(file)
    }

    fn spawn(&self) -> Result<(), String> {
        let log_path = self.dirs.daemon_log();
        if fs::metadata(&log_path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            let _ = fs::remove_file(&log_path);
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("cannot open {}: {e}", log_path.display()))?;
        let log_err = log
            .try_clone()
            .map_err(|e| format!("cannot open {}: {e}", log_path.display()))?;
        let exe = std::env::current_exe().map_err(|e| format!("cannot locate cyamus: {e}"))?;
        let mut child = Command::new(exe)
            .args(["daemon", "run", "--background"])
            .env(cyamus_core::daemon::PORT_VAR, self.port.to_string())
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .map_err(|e| format!("cannot start the daemon: {e}"))?;
        let start = Instant::now();
        loop {
            if probe(self.port).is_some() {
                return Ok(());
            }
            if let Ok(Some(exit)) = child.try_wait() {
                return Err(format!(
                    "daemon exited at startup ({exit}): {}",
                    last_line(&log_path)
                ));
            }
            if start.elapsed() > READY_TIMEOUT {
                return Err(format!(
                    "daemon did not answer on port {} within {READY_TIMEOUT:?}",
                    self.port
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn last_line(path: &std::path::Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| {
            t.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Port from `CYAMUS_DAEMON_PORT`, as an `anyhow` error.
pub fn port() -> anyhow::Result<u16> {
    cyamus_core::daemon::port_from_lookup(|k| std::env::var_os(k))
        .map_err(anyhow::Error::msg)
        .context("invalid daemon port")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyamus_daemon::routes::{Route, Unrouted};
    use cyamus_daemon::status::DockerStatus;

    fn status() -> Status {
        Status {
            pid: 42,
            version: "9.9.9".into(),
            port: 1355,
            docker: DockerStatus {
                reachable: true,
                endpoint: "/var/run/docker.sock".into(),
                source: "default socket".into(),
                error: None,
            },
            routes: vec![Route {
                host: "web.feat-x.myproj.localhost".into(),
                target: "127.0.0.1:49321".parse().unwrap(),
                project: "myproj".into(),
                workspace: Some("feat-x".into()),
                service: "web".into(),
                container: "feat-x-web-1".into(),
            }],
            unrouted: vec![Unrouted {
                container: "feat-x-db-1".into(),
                project: Some("myproj".into()),
                workspace: Some("feat-x".into()),
                reason: "opted out (cyamus.enable=false)".into(),
            }],
        }
    }

    #[test]
    fn renders_without_redirect() {
        assert_eq!(
            render(&status(), false),
            "cyamus daemon 9.9.9 (pid 42) on port 1355\n\
             docker: connected to /var/run/docker.sock (default socket)\n\
             port 80: not redirected; run `cyamus daemon install` to drop :1355 from URLs\n\
             routes:\n  \
             http://web.feat-x.myproj.localhost:1355/  ->  127.0.0.1:49321  (feat-x-web-1)\n\
             not routed:\n  \
             feat-x-db-1 (myproj/feat-x): opted out (cyamus.enable=false)\n"
        );
    }

    #[test]
    fn renders_with_redirect() {
        let out = render(&status(), true);
        assert!(out.contains("port 80: redirected to the daemon (URLs need no port)\n"));
        assert!(out.contains("  http://web.feat-x.myproj.localhost/  ->  127.0.0.1:49321"));
    }

    /// Serves one `/api/status` response with `pid` on a free port.
    fn fake_daemon(pid: u32) -> u16 {
        use std::net::TcpListener;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::to_string(&Status { pid, ..status() }).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        port
    }

    #[test]
    fn detects_redirect_by_pid() {
        let same = fake_daemon(42);
        assert!(redirect_active_via(same, &status()));
        let other = fake_daemon(7);
        assert!(!redirect_active_via(other, &status()));
        let closed = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!redirect_active_via(closed, &status()));
    }
}
