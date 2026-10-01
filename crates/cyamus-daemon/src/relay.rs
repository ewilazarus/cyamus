//! The port-80 relay: a byte-level TCP forwarder from loopback port 80 to the
//! daemon port, installed as a system service by `cyamus daemon install`.
//!
//! It never parses HTTP; each accepted connection is copied, both ways, to the
//! same loopback address on the target port.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

use crate::log;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Forwards connections from `listeners` to `target_port` on the same
/// loopback address until SIGINT or SIGTERM.
pub fn serve(listeners: Vec<std::net::TcpListener>, target_port: u16) -> io::Result<()> {
    if listeners.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no listening sockets",
        ));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        for listener in listeners {
            listener.set_nonblocking(true)?;
            let listener = TcpListener::from_std(listener)?;
            let from = listener.local_addr()?;
            let to = SocketAddr::new(from.ip(), target_port);
            log(&format!("relay: forwarding {from} -> {to}"));
            tokio::spawn(forward(listener, to));
        }
        crate::wait_for_shutdown().await;
        Ok::<(), io::Error>(())
    })?;
    runtime.shutdown_timeout(Duration::from_millis(200));
    Ok(())
}

async fn forward(listener: TcpListener, to: SocketAddr) {
    loop {
        let (mut client, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                log(&format!("relay: accept failed: {e}"));
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        tokio::spawn(async move {
            // A refused or slow target just closes the client: there is no
            // HTTP here to put an error page in.
            let Ok(Ok(mut target)) =
                tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(to)).await
            else {
                return;
            };
            let _ = client.set_nodelay(true);
            let _ = target.set_nodelay(true);
            let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
        });
    }
}
