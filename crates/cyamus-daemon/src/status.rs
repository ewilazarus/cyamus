//! The `/api/status` document, shared by the daemon and `cyamus daemon status`.

use serde::{Deserialize, Serialize};

use crate::routes::{Route, Unrouted};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    pub version: String,
    pub port: u16,
    pub docker: DockerStatus,
    pub routes: Vec<Route>,
    pub unrouted: Vec<Unrouted>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerStatus {
    pub reachable: bool,
    /// Socket path; empty when the endpoint could not be resolved.
    pub endpoint: String,
    /// How the endpoint was chosen, e.g. `Docker context "orbstack"`.
    pub source: String,
    pub error: Option<String>,
}

impl Status {
    /// `http://<host><suffix>/` for a route, where `suffix` is empty when
    /// port 80 reaches the daemon and `:<port>` otherwise.
    pub fn url(&self, route: &Route, suffix: &str) -> String {
        format!("http://{}{suffix}/", route.host)
    }
}
