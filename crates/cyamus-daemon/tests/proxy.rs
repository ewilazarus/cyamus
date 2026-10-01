//! proxy-routing spec, end to end against real local backends (no Docker:
//! the container snapshot is injected).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cyamus_daemon::docker::{Endpoint, Snapshot};
use cyamus_daemon::routes::{COMPOSE_SERVICE, COMPOSE_WORKING_DIR, Container, PortBinding};
use cyamus_daemon::state::State;
use cyamus_daemon::{Status, server};
use cyamus_registry::{Record, Registry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

const HOST: &str = "web.feat-x.myproj.localhost";

struct Harness {
    _tmp: tempfile::TempDir,
    proxy: SocketAddr,
}

/// Starts a proxy whose only route sends `web.feat-x.myproj.localhost` to
/// `backend`.
async fn harness(backend: SocketAddr) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let wt = root.join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    let registry = Registry::new(root.join("workspaces"), root.join("registry.lock"));
    registry
        .register(&Record {
            project: "myproj".into(),
            branch: "feat/x".into(),
            label: "feat-x".into(),
            path: wt.clone(),
        })
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let endpoint = Endpoint {
        socket: Ok(PathBuf::from("/nonexistent.sock")),
        source: "test".into(),
    };
    let state = Arc::new(State::new(registry, endpoint, proxy.port()));
    state.set_docker(Snapshot {
        reachable: true,
        error: None,
        containers: vec![Container {
            id: "c1".into(),
            name: "feat-x-web-1".into(),
            labels: HashMap::from([
                (COMPOSE_WORKING_DIR.into(), wt.display().to_string()),
                (COMPOSE_SERVICE.into(), "web".into()),
            ]),
            ports: vec![PortBinding {
                container_port: 3000,
                host_ip: Some("0.0.0.0".into()),
                host_port: Some(backend.port()),
                tcp: true,
            }],
            created: 1,
        }],
    });
    tokio::spawn(server::serve(listener, state));
    Harness { _tmp: tmp, proxy }
}

/// Sends a raw HTTP/1.1 request and returns the whole response as text.
async fn raw(addr: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out))
        .await
        .expect("response within 5s")
        .unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

fn get(host: &str, path: &str, extra: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n")
}

/// A backend that answers every request with the request head it received.
async fn echo_head_backend() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let head = read_head(&mut s).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\nX-Backend: yes\r\n\r\n{head}",
                    head.len()
                );
                s.write_all(resp.as_bytes()).await.unwrap();
            });
        }
    });
    addr
}

async fn read_head(s: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).await.unwrap() == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn forwards_request_with_headers() {
    let backend = echo_head_backend().await;
    let h = harness(backend).await;
    let host = format!("Web.Feat-X.myproj.localhost:{}", h.proxy.port());
    let resp = raw(
        h.proxy,
        &get(
            &host,
            "/api/items?x=1",
            "X-Forwarded-For: 10.0.0.9\r\nKeep-Alive: timeout=5\r\n",
        ),
    )
    .await;
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.contains("x-backend: yes"), "{resp}");
    let body = resp.split("\r\n\r\n").nth(1).unwrap().to_ascii_lowercase();
    assert!(body.starts_with("get /api/items?x=1 http/1.1"), "{body}");
    assert!(
        body.contains(&format!("host: {}", host.to_ascii_lowercase())),
        "{body}"
    );
    assert!(
        body.contains(&format!("x-forwarded-host: {}", host.to_ascii_lowercase())),
        "{body}"
    );
    assert!(body.contains("x-forwarded-proto: http"), "{body}");
    assert!(
        body.contains("x-forwarded-for: 10.0.0.9, 127.0.0.1"),
        "{body}"
    );
    assert!(!body.contains("keep-alive: timeout"), "{body}");
}

#[tokio::test]
async fn streams_response_incrementally() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = listener.local_addr().unwrap();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        read_head(&mut s).await;
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        s.write_all(b"d\r\ndata: first\n\n\r\n").await.unwrap();
        // The second event is only sent once the client saw the first.
        release_rx.await.unwrap();
        s.write_all(b"e\r\ndata: second\n\n\r\n0\r\n\r\n")
            .await
            .unwrap();
    });
    let h = harness(backend).await;
    let mut client = TcpStream::connect(h.proxy).await.unwrap();
    client
        .write_all(get(HOST, "/events", "").as_bytes())
        .await
        .unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !String::from_utf8_lossy(&seen).contains("data: first") {
        let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("first event arrives before the response completes")
            .unwrap();
        assert!(n > 0, "connection closed early");
        seen.extend_from_slice(&buf[..n]);
    }
    assert!(!String::from_utf8_lossy(&seen).contains("data: second"));
    release_tx.send(()).unwrap();
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).await.unwrap();
    assert!(String::from_utf8_lossy(&rest).contains("data: second"));
}

#[tokio::test]
async fn relays_protocol_upgrades() {
    // Minimal upgrade server: 101, then echo bytes upper-cased.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let head = read_head(&mut s).await.to_ascii_lowercase();
        assert!(head.contains("upgrade: websocket"), "{head}");
        assert!(head.contains("connection: upgrade"), "{head}");
        s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").await.unwrap();
        let mut buf = [0u8; 64];
        loop {
            let n = s.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            let upper: Vec<u8> = buf[..n].to_ascii_uppercase();
            s.write_all(&upper).await.unwrap();
        }
    });
    let h = harness(backend).await;
    let mut client = TcpStream::connect(h.proxy).await.unwrap();
    client
        .write_all(format!("GET /hmr HTTP/1.1\r\nHost: {HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let head = read_head(&mut client).await;
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    for msg in ["ping", "hot update"] {
        client.write_all(msg.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; msg.len()];
        tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(buf, msg.to_ascii_uppercase().as_bytes());
    }
}

#[tokio::test]
async fn unknown_host_lists_routes() {
    let h = harness(echo_head_backend().await).await;
    let typo = format!("wbe.feat-x.myproj.localhost:{}", h.proxy.port());
    let resp = raw(h.proxy, &get(&typo, "/", "")).await;
    assert!(resp.starts_with("HTTP/1.1 404"), "{resp}");
    assert!(
        resp.contains("no route for wbe.feat-x.myproj.localhost"),
        "{resp}"
    );
    assert!(
        resp.contains(&format!("http://{HOST}:{}/", h.proxy.port())),
        "{resp}"
    );

    let html = raw(
        h.proxy,
        &get("wbe.feat-x.myproj.localhost", "/", "Accept: text/html\r\n"),
    )
    .await;
    assert!(
        html.contains("text/html") && html.contains("<code>wbe.feat-x.myproj.localhost</code>"),
        "{html}"
    );
}

#[tokio::test]
async fn refused_backend_is_bad_gateway() {
    // Bind then drop to get a port nothing listens on.
    let dead = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let h = harness(dead).await;
    let resp = raw(h.proxy, &get(HOST, "/", "")).await;
    assert!(resp.starts_with("HTTP/1.1 502"), "{resp}");
    assert!(
        resp.contains(HOST) && resp.contains(&dead.to_string()),
        "{resp}"
    );
}

#[tokio::test]
async fn status_api_and_route_page() {
    let backend = echo_head_backend().await;
    let h = harness(backend).await;
    let resp = raw(h.proxy, &get("cyamus.localhost", "/api/status", "")).await;
    assert!(resp.contains("application/json"), "{resp}");
    let json = resp.split("\r\n\r\n").nth(1).unwrap();
    let status: Status = serde_json::from_str(json).unwrap();
    assert_eq!(status.port, h.proxy.port());
    assert_eq!(status.version, cyamus_daemon::VERSION);
    assert_eq!(status.routes.len(), 1);
    assert_eq!(status.routes[0].host, HOST);
    assert_eq!(
        status.routes[0].target,
        SocketAddr::from(([127, 0, 0, 1], backend.port()))
    );

    // Links follow the port the request came in on.
    let admin = format!("cyamus.localhost:{}", h.proxy.port());
    let page = raw(h.proxy, &get(&admin, "/", "")).await;
    let url = format!("http://{HOST}:{}/", h.proxy.port());
    assert!(
        page.contains(&format!("<a href=\"{url}\">{url}</a>")),
        "{page}"
    );
    // A port-less Host, as through the port-80 redirect: port-less links.
    let page = raw(h.proxy, &get("cyamus.localhost", "/", "")).await;
    let url = format!("http://{HOST}/");
    assert!(
        page.contains(&format!("<a href=\"{url}\">{url}</a>")),
        "{page}"
    );
    let missing = raw(h.proxy, &get("nope.feat-x.myproj.localhost", "/", "")).await;
    assert!(missing.contains(&format!("http://{HOST}/ ->")), "{missing}");
}
