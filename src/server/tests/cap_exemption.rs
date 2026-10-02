//! The per-peer connection caps over the real accept path: a `SniffListener` decides the exemption
//! (`Config::is_cap_exempt`) and `serve_router` applies the live caps with it. Peers are scripted
//! through a `TcpAccept` source because a real loopback client cannot be anything but loopback.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;
use shadowcat::config::Config;
use shadowcat::http::conn_serve::{serve_router, ConnLimits, ServeOutcome};
use shadowcat::tls::listener::{Limits, SniffListener, TcpAccept};
use shadowcat::tls::TlsRuntime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

/// A TCP source that reports one fixed peer address for every connection.
struct FixedPeer {
    /// The real listener.
    inner: TcpListener,
    /// The address reported as every connection's peer.
    peer: SocketAddr,
}

impl TcpAccept for FixedPeer {
    async fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        let (sock, _real) = self.inner.accept().await?;
        Ok((sock, self.peer))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// What a running server exposes to a test.
struct Running {
    /// The listener's address.
    addr: SocketAddr,
    /// Ends the serve loop when dropped or fired.
    _shutdown: tokio::sync::oneshot::Sender<()>,
    /// The serve loop.
    _task: tokio::task::JoinHandle<ServeOutcome>,
}

/// Serves `/ok` (and `/endless`, a response that never ends) behind a sniffing listener that reports every peer as `peer_ip`, with the live caps
/// in `limits`. The admission caps are far above every test's connection count, so any refusal
/// under test is the live cap.
async fn serve(peer_ip: &str, limits: ConnLimits) -> Running {
    shadowcat::tls::install_default_provider();
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap();
    let peer: SocketAddr = if peer_ip.contains(':') {
        format!("[{peer_ip}]:4000")
    } else {
        format!("{peer_ip}:4000")
    }
    .parse()
    .unwrap();
    let config = Arc::new(Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    });
    let listener = SniffListener::new(
        FixedPeer { inner: tcp, peer },
        TlsRuntime::start(&config),
        config,
        Limits {
            global_cap: 100,
            per_ip_cap: 100,
            deadline: Duration::from_secs(30),
        },
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_router(
        listener,
        Router::new().route("/ok", get(|| async { "ok" })).route(
            "/endless",
            get(|| async {
                axum::body::Body::from_stream(futures_util::stream::repeat_with(|| {
                    Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![0u8; 16 * 1024]))
                }))
            }),
        ),
        async move {
            let _ = rx.await;
        },
        limits,
        || {},
    ));
    Running {
        addr,
        _shutdown: tx,
        _task: task,
    }
}

/// Live caps with the given bounds and the default time bounds.
fn caps(max_live: usize, max_live_per_peer: u32) -> ConnLimits {
    ConnLimits {
        max_live,
        max_live_per_peer,
        ..ConnLimits::default()
    }
}

/// Opens a connection, completes one keep-alive request and returns it still open.
async fn open_served(addr: SocketAddr) -> TcpStream {
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 512];
    let n = timeout(Duration::from_secs(5), c.read(&mut buf))
        .await
        .expect("the connection was not served")
        .expect("the connection failed");
    assert!(
        buf[..n].starts_with(b"HTTP/1.1 200"),
        "not served: {n} bytes"
    );
    c
}

/// How a new connection ended: the number of response bytes it received before the peer closed
/// it (EOF or reset), or `None` when it stayed open past the bound.
async fn bytes_before_close(addr: SocketAddr) -> Option<usize> {
    let mut c = TcpStream::connect(addr).await.unwrap();
    let _ = c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let mut total = 0;
    let mut buf = [0u8; 512];
    loop {
        match timeout(Duration::from_secs(3), c.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return Some(total),
            Ok(Ok(n)) => total += n,
            Err(_) => return None,
        }
    }
}

#[tokio::test]
async fn loopback_peers_are_not_refused_by_the_live_per_peer_cap_but_the_global_cap_still_binds() {
    for loopback in ["127.0.0.1", "::ffff:127.0.0.1"] {
        let server = serve(loopback, caps(3, 2)).await;
        let _a = open_served(server.addr).await;
        let _b = open_served(server.addr).await;
        let _c = open_served(server.addr).await;
        assert_eq!(
            bytes_before_close(server.addr).await,
            Some(0),
            "{loopback}: the fourth connection must be refused by the global cap"
        );
    }
}

#[tokio::test]
async fn an_ordinary_peer_is_refused_by_the_live_per_peer_cap() {
    let server = serve("192.0.2.1", caps(3, 2)).await;
    let _a = open_served(server.addr).await;
    let _b = open_served(server.addr).await;
    assert_eq!(
        bytes_before_close(server.addr).await,
        Some(0),
        "the third connection from one ordinary peer must be refused at accept"
    );
}

#[tokio::test]
async fn a_refused_connection_is_closed_unread_and_a_closed_one_releases_its_slot() {
    let server = serve("192.0.2.1", caps(100, 1)).await;
    let first = open_served(server.addr).await;
    // Refused at accept: closed with no response bytes, never left to time out.
    assert_eq!(bytes_before_close(server.addr).await, Some(0));
    drop(first);
    // The release happens when the connection task ends, after the socket closes, so a single
    // immediate attempt is not deterministic: retry within a bound.
    timeout(Duration::from_secs(10), async {
        loop {
            if bytes_before_close(server.addr).await != Some(0) {
                return;
            }
        }
    })
    .await
    .expect("a closed connection never released its slot");
}

#[tokio::test]
async fn a_stall_cut_releases_a_peers_slot_while_its_client_stays_connected() {
    let server = serve(
        "192.0.2.1",
        ConnLimits {
            egress_window: Duration::from_secs(1),
            ..caps(100, 1)
        },
    )
    .await;
    // The only slot is taken by a connection whose client asks for an endless response and then
    // never reads it: the server's writer parks and the connection unit cuts it.
    let mut stalled = TcpStream::connect(server.addr).await.unwrap();
    stalled
        .write_all(b"GET /endless HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    assert_eq!(
        bytes_before_close(server.addr).await,
        Some(0),
        "a second connection must be refused while the stalled one holds the slot"
    );
    // The slot comes back through the cut, not through the client: `stalled` is still open here.
    timeout(Duration::from_secs(20), async {
        loop {
            if bytes_before_close(server.addr).await != Some(0) {
                return;
            }
        }
    })
    .await
    .expect("a stall cut never released its connection's slot");
    drop(stalled);
}
