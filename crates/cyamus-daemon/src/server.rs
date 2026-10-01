//! The HTTP side: accepting connections, forwarding requests (WebSocket
//! upgrades included), and the daemon's own pages.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};

use crate::host::{self, Target};
use crate::routes::{Route, Table};
use crate::state::State;

pub type Body = BoxBody<Bytes, hyper::Error>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts connections on `listener` forever.
pub async fn serve(listener: TcpListener, state: Arc<State>) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                crate::log(&format!("accept failed: {e}"));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let service = service_fn(move |req| handle(req, peer, Arc::clone(&state)));
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

async fn handle(
    req: Request<Incoming>,
    peer: SocketAddr,
    state: Arc<State>,
) -> Result<Response<Body>, Infallible> {
    let raw_host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
        .unwrap_or_default();
    // Links point back through whatever port this request came in on, so a
    // request via the port-80 redirect gets port-less links.
    let links = host::port_suffix(&raw_host);
    let response = match host::classify(&raw_host) {
        Target::Admin => admin(&req, &state, &links),
        Target::Route(host) => {
            let table = state.table();
            match table.get(&host).cloned() {
                Some(route) => proxy(req, &route, peer).await,
                None => not_found(&req, &host, &table, &links),
            }
        }
    };
    Ok(response)
}

// --- forwarding -------------------------------------------------------------

/// Headers that describe a single hop and must not be forwarded (RFC 9110
/// §7.6.1), besides any named in `Connection`.
const HOP_BY_HOP: [&str; 7] = [
    "connection",
    "proxy-connection",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in listed {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
}

fn upgrade_requested(headers: &HeaderMap) -> Option<HeaderValue> {
    let connection_upgrade = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"));
    connection_upgrade
        .then(|| headers.get(header::UPGRADE).cloned())
        .flatten()
}

async fn proxy(mut req: Request<Incoming>, route: &Route, peer: SocketAddr) -> Response<Body> {
    let upgrade = upgrade_requested(req.headers());
    let client_upgrade = upgrade.is_some().then(|| hyper::upgrade::on(&mut req));

    let (mut parts, body) = req.into_parts();
    let original_host = parts.headers.get(header::HOST).cloned();
    strip_hop_by_hop(&mut parts.headers);
    if let Some(protocol) = &upgrade {
        parts
            .headers
            .insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        parts.headers.insert(header::UPGRADE, protocol.clone());
    }
    if let Some(host) = &original_host {
        parts.headers.insert("x-forwarded-host", host.clone());
    }
    parts
        .headers
        .insert("x-forwarded-proto", HeaderValue::from_static("http"));
    let forwarded_for = match parts
        .headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        Some(prior) => format!("{prior}, {}", peer.ip()),
        None => peer.ip().to_string(),
    };
    if let Ok(value) = HeaderValue::from_str(&forwarded_for) {
        parts.headers.insert("x-forwarded-for", value);
    }
    parts.uri = parts.uri.path_and_query().map_or_else(
        || "/".parse().expect("valid"),
        |pq| pq.as_str().parse().expect("valid"),
    );
    parts.version = Version::HTTP_11;

    let stream = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(route.target)).await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return bad_gateway(route, &e.to_string()),
        Err(_) => return bad_gateway(route, "connection timed out"),
    };
    let _ = stream.set_nodelay(true);
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
    {
        Ok(pair) => pair,
        Err(e) => return bad_gateway(route, &e.to_string()),
    };
    tokio::spawn(conn.with_upgrades());

    let mut response = match sender.send_request(Request::from_parts(parts, body)).await {
        Ok(response) => response,
        Err(e) => return bad_gateway(route, &e.to_string()),
    };

    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client_upgrade) = client_upgrade
    {
        let backend_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            if let (Ok(client), Ok(backend)) = tokio::join!(client_upgrade, backend_upgrade) {
                let mut client = TokioIo::new(client);
                let mut backend = TokioIo::new(backend);
                let _ = tokio::io::copy_bidirectional(&mut client, &mut backend).await;
            }
        });
    } else {
        strip_hop_by_hop(response.headers_mut());
    }
    response.map(BodyExt::boxed)
}

// --- generated responses ----------------------------------------------------

fn full(text: impl Into<Bytes>) -> Body {
    Full::new(text.into())
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn respond(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
) -> Response<Body> {
    let mut response = Response::new(full(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn wants_html<B>(req: &Request<B>) -> bool {
    req.headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"))
}

fn bad_gateway(route: &Route, error: &str) -> Response<Body> {
    respond(
        StatusCode::BAD_GATEWAY,
        "text/plain; charset=utf-8",
        format!(
            "cyamus: {} is routed to {} (container {}), but it is not answering: {error}\n",
            route.host, route.target, route.container
        ),
    )
}

fn not_found<B>(req: &Request<B>, host: &str, table: &Table, links: &str) -> Response<Body> {
    if wants_html(req) {
        let body = page(
            "No route",
            &format!(
                "<p>Nothing is routed at <code>{}</code>.</p>{}",
                escape(host),
                routes_html(table, links)
            ),
        );
        return respond(StatusCode::NOT_FOUND, "text/html; charset=utf-8", body);
    }
    let mut text = format!("cyamus: no route for {host}\n");
    if table.routes.is_empty() {
        text.push_str("There are no routes.\n");
    } else {
        text.push_str("Routes:\n");
        for route in table.routes.values() {
            text.push_str(&format!(
                "  http://{}{links}/ -> {}\n",
                route.host, route.target
            ));
        }
    }
    respond(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", text)
}

fn admin<B>(req: &Request<B>, state: &State, links: &str) -> Response<Body> {
    match req.uri().path() {
        "/api/status" => {
            let json = serde_json::to_vec_pretty(&state.status()).unwrap_or_default();
            respond(StatusCode::OK, "application/json", json)
        }
        "/" => {
            let status = state.status();
            let docker = if status.docker.reachable {
                format!(
                    "connected to <code>{}</code>",
                    escape(&status.docker.endpoint)
                )
            } else {
                format!(
                    "not reachable ({})",
                    escape(status.docker.error.as_deref().unwrap_or("connecting"))
                )
            };
            let table = state.table();
            let body = page(
                "cyamus",
                &format!(
                    "<p>Daemon {} (pid {}) on port {}. Docker: {docker}.</p>{}",
                    escape(&status.version),
                    status.pid,
                    status.port,
                    routes_html(&table, links)
                ),
            );
            respond(StatusCode::OK, "text/html; charset=utf-8", body)
        }
        _ => respond(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            "not found\n",
        ),
    }
}

fn routes_html(table: &Table, links: &str) -> String {
    let mut html = String::from("<h2>Routes</h2>");
    if table.routes.is_empty() {
        html.push_str("<p>No routes.</p>");
    } else {
        html.push_str("<table><tr><th>URL</th><th>Target</th><th>Container</th></tr>");
        for r in table.routes.values() {
            let url = format!("http://{}{links}/", r.host);
            html.push_str(&format!(
                "<tr><td><a href=\"{u}\">{u}</a></td><td>{}</td><td>{}</td></tr>",
                r.target,
                escape(&r.container),
                u = escape(&url)
            ));
        }
        html.push_str("</table>");
    }
    if !table.unrouted.is_empty() {
        html.push_str("<h2>Not routed</h2><table><tr><th>Container</th><th>Reason</th></tr>");
        for u in &table.unrouted {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                escape(&u.container),
                escape(&u.reason)
            ));
        }
        html.push_str("</table>");
    }
    html
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:15px/1.5 system-ui,sans-serif;margin:2rem;max-width:60rem}}\
         table{{border-collapse:collapse}}td,th{{padding:.25rem .75rem;text-align:left;border-bottom:1px solid #8884}}\
         code{{font-size:90%}}</style></head><body><h1>{title}</h1>{body}</body></html>"
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_by_hop_headers_are_stripped() {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-secret"),
        );
        h.insert("x-secret", HeaderValue::from_static("1"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        h.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        strip_hop_by_hop(&mut h);
        assert_eq!(h.len(), 1);
        assert!(h.contains_key(header::ACCEPT));
    }

    #[test]
    fn detects_upgrade() {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, Upgrade"),
        );
        h.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        assert_eq!(
            upgrade_requested(&h),
            Some(HeaderValue::from_static("websocket"))
        );
        h.remove(header::CONNECTION);
        assert_eq!(upgrade_requested(&h), None);
    }

    #[test]
    fn escaping() {
        assert_eq!(
            escape("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
    }
}
