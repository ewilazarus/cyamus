//! What core needs from the routing daemon, without depending on it.
//!
//! Starting the daemon means spawning the `cyamus` binary, which is the CLI's
//! business; core only asks for it through [`DaemonControl`].

use std::ffi::OsString;

/// Port the daemon listens on when `CYAMUS_DAEMON_PORT` is unset.
pub const DEFAULT_PORT: u16 = 1355;

/// Environment variable that overrides the daemon port.
pub const PORT_VAR: &str = "CYAMUS_DAEMON_PORT";

/// Reads the daemon port from `CYAMUS_DAEMON_PORT` via `lookup`.
pub fn port_from_lookup(lookup: impl Fn(&str) -> Option<OsString>) -> Result<u16, String> {
    match lookup(PORT_VAR) {
        None => Ok(DEFAULT_PORT),
        Some(raw) => raw
            .to_str()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|p| *p != 0)
            .ok_or_else(|| format!("{PORT_VAR} must be a port number (1-65535), got {raw:?}")),
    }
}

/// Starts and locates the routing daemon on behalf of `workspace setup`.
pub trait DaemonControl {
    /// The port the daemon listens on, exposed to hooks as `CYAMUS_PROXY_PORT`.
    fn port(&self) -> u16;

    /// Makes sure an up-to-date daemon is running. Failures are reported to
    /// the user as warnings and never fail setup.
    fn ensure_running(&mut self) -> Result<(), String>;

    /// What follows a hostname in a URL: empty when loopback port 80 reaches
    /// the daemon, `:<port>` otherwise. Exposed as `CYAMUS_URL_SUFFIX`.
    fn url_suffix(&self) -> String {
        format!(":{}", self.port())
    }
}

/// A [`DaemonControl`] that never starts anything.
#[derive(Debug, Clone, Copy)]
pub struct NoDaemon {
    pub port: u16,
}

impl Default for NoDaemon {
    fn default() -> Self {
        Self { port: DEFAULT_PORT }
    }
}

impl DaemonControl for NoDaemon {
    fn port(&self) -> u16 {
        self.port
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_resolution() {
        assert_eq!(port_from_lookup(|_| None), Ok(DEFAULT_PORT));
        assert_eq!(port_from_lookup(|_| Some("4000".into())), Ok(4000));
        assert!(port_from_lookup(|_| Some("0".into())).is_err());
        assert!(port_from_lookup(|_| Some("http".into())).is_err());
        assert!(port_from_lookup(|_| Some("70000".into())).is_err());
    }
}
