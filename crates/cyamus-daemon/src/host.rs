//! Interpreting the `Host` of an incoming request.

/// The daemon's own hostname (route page and status API).
pub const ADMIN_HOST: &str = "cyamus.localhost";

/// What a request's host refers to.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// The daemon itself.
    Admin,
    /// A candidate route hostname, normalized for lookup.
    Route(String),
}

/// Normalizes a `Host` header value: lowercased, port and trailing dot
/// removed. IPv6 literals keep their brackets.
pub fn normalize(raw: &str) -> String {
    let raw = raw.trim();
    let host = if raw.starts_with('[') {
        raw.split_inclusive(']').next().unwrap_or(raw)
    } else {
        raw.rsplit_once(':').map_or(raw, |(h, _)| h)
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The port part of a `Host` header as a URL suffix (`":1355"`), or empty
/// when the request came in on the default port.
pub fn port_suffix(raw: &str) -> String {
    let raw = raw.trim();
    let port = if raw.starts_with('[') {
        raw.split_once("]:").map_or("", |(_, port)| port)
    } else {
        raw.rsplit_once(':').map_or("", |(_, port)| port)
    };
    match port.parse::<u16>() {
        Ok(80) | Err(_) => String::new(),
        Ok(p) => format!(":{p}"),
    }
}

pub fn classify(raw: &str) -> Target {
    let host = normalize(raw);
    if host == ADMIN_HOST {
        Target::Admin
    } else {
        Target::Route(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization() {
        assert_eq!(
            normalize("Web.Feat-X.myproj.localhost:1355"),
            "web.feat-x.myproj.localhost"
        );
        assert_eq!(
            normalize("web.feat-x.myproj.localhost."),
            "web.feat-x.myproj.localhost"
        );
        assert_eq!(
            normalize("web.feat-x.myproj.localhost.:80"),
            "web.feat-x.myproj.localhost"
        );
        assert_eq!(normalize("[::1]:1355"), "[::1]");
        assert_eq!(normalize("localhost"), "localhost");
    }

    #[test]
    fn port_suffixes() {
        assert_eq!(port_suffix("cyamus.localhost:1355"), ":1355");
        assert_eq!(port_suffix("cyamus.localhost"), "");
        assert_eq!(port_suffix("cyamus.localhost:80"), "");
        assert_eq!(port_suffix("[::1]:1355"), ":1355");
        assert_eq!(port_suffix("[::1]"), "");
    }

    #[test]
    fn admin_host() {
        assert_eq!(classify("Cyamus.localhost:1355"), Target::Admin);
        assert_eq!(
            classify("web.feat-x.myproj.localhost"),
            Target::Route("web.feat-x.myproj.localhost".into())
        );
    }
}
