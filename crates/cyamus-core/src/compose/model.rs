//! The parts of `docker compose config --format json` cyamus needs.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Service {
    /// Normalized long form: `{name: {condition, required, restart}}`.
    pub depends_on: Map<String, Value>,
    /// `None` for services with a `network_mode`; an implicit default shows
    /// up as `{"default": null}`.
    pub networks: Option<Map<String, Value>>,
    pub network_mode: Option<String>,
    /// Profiles that enable the service; empty when it always runs.
    pub profiles: Vec<String>,
}

impl Model {
    /// Services that start without activating a profile.
    pub fn default_services(&self) -> impl Iterator<Item = &str> {
        self.services
            .iter()
            .filter(|(_, s)| s.profiles.is_empty())
            .map(|(name, _)| name.as_str())
    }
}

pub fn parse(json: &str) -> Result<Model, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("unreadable compose config: {e}"))?;
    let services = root
        .get("services")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let services = services
        .into_iter()
        .map(|(name, s)| {
            let object = |key: &str| s.get(key).and_then(Value::as_object).cloned();
            let service = Service {
                depends_on: object("depends_on").unwrap_or_default(),
                networks: object("networks"),
                network_mode: s
                    .get("network_mode")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                profiles: s
                    .get("profiles")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            (name, service)
        })
        .collect();
    Ok(Model { services })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from Compose 5.1.2 (`config --format json --profile '*'`).
    const FIXTURE: &str = r#"{
      "name": "m",
      "services": {
        "web": {"image": "alpine:3", "networks": {"default": null},
                "depends_on": {"cache": {"condition": "service_started", "required": false},
                               "db": {"condition": "service_healthy", "restart": true, "required": true}}},
        "worker": {"networks": {"backend": {"aliases": ["jobs"]}},
                   "depends_on": {"cache": {"condition": "service_started", "required": true}}},
        "hostnet": {"network_mode": "host"},
        "sidecar": {"network_mode": "service:web",
                    "depends_on": {"web": {"condition": "service_started", "restart": true, "required": true}}},
        "debug": {"profiles": ["debug"], "networks": {"default": null}}
      },
      "networks": {"backend": {"name": "m_backend"}, "default": {"name": "m_default"}}
    }"#;

    #[test]
    fn parses_the_relevant_parts() {
        let m = parse(FIXTURE).unwrap();
        assert_eq!(m.services.len(), 5);
        let web = &m.services["web"];
        assert_eq!(web.depends_on.keys().collect::<Vec<_>>(), ["cache", "db"]);
        assert_eq!(web.networks.as_ref().unwrap()["default"], Value::Null);
        assert_eq!(
            m.services["worker"].networks.as_ref().unwrap()["backend"]["aliases"][0],
            "jobs"
        );
        assert_eq!(m.services["hostnet"].network_mode.as_deref(), Some("host"));
        assert!(m.services["hostnet"].networks.is_none());
        assert!(m.services.contains_key("debug"));
        assert_eq!(m.services["debug"].profiles, ["debug"]);
        let defaults: Vec<&str> = m.default_services().collect();
        assert!(!defaults.contains(&"debug") && defaults.contains(&"web"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("not json").is_err());
    }
}
