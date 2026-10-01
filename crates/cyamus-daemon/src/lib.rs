//! The cyamus routing daemon: a loopback-only HTTP reverse proxy whose routes
//! come from the workspace registry and from running Docker containers.
//!
//! This is the only crate that uses async code; `cyamus-core` stays
//! synchronous. [`run`] owns the tokio runtime, so callers stay synchronous
//! too.

pub mod docker;
pub mod host;
pub mod relay;
pub mod routes;
pub mod server;
pub mod state;
pub mod status;

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cyamus_core::paths::Dirs;
use cyamus_registry::Registry;

pub use status::Status;

use crate::state::State;

/// Daemon version, compared by `workspace setup` to decide on a handover.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("a cyamus daemon is already running{}", pid.map(|p| format!(" (pid {p})")).unwrap_or_default())]
    AlreadyRunning { pid: Option<u32> },
    #[error(
        "cannot listen on port {port} (127.0.0.1: {v4}; ::1: {v6}); set CYAMUS_DAEMON_PORT to use another port"
    )]
    PortUnavailable {
        port: u16,
        v4: io::Error,
        v6: io::Error,
    },
    #[error("{action} {path}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Runs the daemon in the foreground until SIGINT or SIGTERM.
pub fn run(
    dirs: &Dirs,
    port: u16,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<(), DaemonError> {
    let state_dir = dirs.state_dir();
    fs::create_dir_all(&state_dir).map_err(|source| DaemonError::Io {
        action: "cannot create",
        path: state_dir.clone(),
        source,
    })?;
    let _lock = acquire_lock(dirs)?;
    let listeners = bind(port)?;
    let pid_file = dirs.daemon_pid();
    fs::write(&pid_file, format!("{}\n", std::process::id())).map_err(|source| {
        DaemonError::Io {
            action: "cannot write",
            path: pid_file.clone(),
            source,
        }
    })?;

    let endpoint = docker::resolve_endpoint(&env);
    let registry = Registry::new(dirs.registry_dir(), dirs.registry_lock());
    let state = Arc::new(State::new(registry, endpoint.clone(), port));
    let addrs: Vec<String> = listeners
        .iter()
        .filter_map(|l| l.local_addr().ok())
        .map(|a| a.to_string())
        .collect();
    log(&format!(
        "cyamus daemon {VERSION} (pid {}) listening on {}; docker via {}",
        std::process::id(),
        addrs.join(" and "),
        endpoint.source
    ));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Io {
            action: "cannot start the async runtime for",
            path: state_dir,
            source,
        })?;
    runtime.block_on(async {
        tokio::spawn(docker::watch(endpoint, Arc::clone(&state)));
        for listener in listeners {
            match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => {
                    tokio::spawn(server::serve(listener, Arc::clone(&state)));
                }
                Err(e) => log(&format!("cannot register listener: {e}")),
            }
        }
        wait_for_shutdown().await;
    });
    // Open WebSockets and in-flight requests are dropped, not drained.
    runtime.shutdown_timeout(Duration::from_millis(500));
    let _ = fs::remove_file(&pid_file);
    log("stopped");
    Ok(())
}

fn acquire_lock(dirs: &Dirs) -> Result<File, DaemonError> {
    let path = dirs.daemon_lock();
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| DaemonError::Io {
            action: "cannot open",
            path: path.clone(),
            source,
        })?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(DaemonError::AlreadyRunning {
            pid: read_pid(dirs),
        }),
        Err(TryLockError::Error(source)) => Err(DaemonError::Io {
            action: "cannot lock",
            path,
            source,
        }),
    }
}

/// The PID recorded by the running daemon, if any.
pub fn read_pid(dirs: &Dirs) -> Option<u32> {
    fs::read_to_string(dirs.daemon_pid())
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Listening sockets on the two loopback addresses; either may be missing.
#[derive(Debug, Default)]
pub struct Loopback {
    pub v4: Option<std::net::TcpListener>,
    pub v6: Option<std::net::TcpListener>,
}

impl Loopback {
    pub fn into_vec(self) -> Vec<std::net::TcpListener> {
        self.v4.into_iter().chain(self.v6).collect()
    }
}

/// Binds `port` on `127.0.0.1` and `::1` (non-blocking). One failing is
/// tolerated and logged; both failing returns both errors.
pub fn bind_loopback(port: u16) -> Result<Loopback, (io::Error, io::Error)> {
    let open = |addr: SocketAddr| -> io::Result<std::net::TcpListener> {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(listener)
    };
    match (
        open(SocketAddr::from((Ipv4Addr::LOCALHOST, port))),
        open(SocketAddr::from((Ipv6Addr::LOCALHOST, port))),
    ) {
        (Err(v4), Err(v6)) => Err((v4, v6)),
        (v4, v6) => {
            let keep = |name: &str, result: io::Result<std::net::TcpListener>| match result {
                Ok(l) => Some(l),
                Err(e) => {
                    log(&format!("warning: not listening on {name}:{port}: {e}"));
                    None
                }
            };
            Ok(Loopback {
                v4: keep("127.0.0.1", v4),
                v6: keep("[::1]", v6),
            })
        }
    }
}

fn bind(port: u16) -> Result<Vec<std::net::TcpListener>, DaemonError> {
    bind_loopback(port)
        .map(Loopback::into_vec)
        .map_err(|(v4, v6)| DaemonError::PortUnavailable { port, v4, v6 })
}

pub(crate) async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => log("received SIGTERM"),
        _ = int.recv() => log("received SIGINT"),
    }
}

/// Writes a timestamped line to stderr (the daemon log when backgrounded).
pub fn log(message: &str) {
    eprintln!("{} {message}", utc_now());
}

/// `YYYY-MM-DDTHH:MM:SSZ` without a date-time dependency.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_362), (2025, 10, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
