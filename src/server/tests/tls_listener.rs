//! The sniffing listener over real sockets.
//!
//! The tests cover first-byte dispatch, deadlines, admission caps, accept-error continuation and
//! shutdown. The two-phase handshake's hot-swap race is a unit test beside `negotiate_with`.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use shadowcat::config::{Config, TlsMode};
use shadowcat::http::conn_serve::ConnSource;
use shadowcat::tls::listener::{ConnStream, Limits, SniffListener, TcpAccept};
use shadowcat::tls::selfsigned::SanSources;
use shadowcat::tls::{ConnMeta, Selected, TlsRuntime};
use shadowcat_test_support::tls::{handshake_any, sha256_hex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

fn memory_config() -> Config {
    Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    }
}

async fn bind() -> (TcpListener, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap();
    (l, a)
}

async fn listener_with(config: Config, limits: Limits) -> (SniffListener, SocketAddr) {
    let (l, addr, _rt) = listener_with_runtime(config, limits).await;
    (l, addr)
}

async fn listener_with_runtime(
    config: Config,
    limits: Limits,
) -> (SniffListener, SocketAddr, Arc<TlsRuntime>) {
    shadowcat::tls::install_default_provider();
    let (tcp, addr) = bind().await;
    let config = Arc::new(config);
    let rt = TlsRuntime::start(&config);
    (
        SniffListener::new(tcp, rt.clone(), config, limits),
        addr,
        rt,
    )
}

async fn accept(l: &mut SniffListener) -> (ConnStream, ConnMeta) {
    timeout(Duration::from_secs(10), ConnSource::accept(l))
        .await
        .expect("accept timed out")
        .expect("a sniff listener never returns an accept error")
        .expect("the listener ended")
}

/// True when the peer closes (EOF or reset) within `within`.
async fn closes_within(s: &mut TcpStream, within: Duration) -> bool {
    let mut buf = [0u8; 16];
    match timeout(within, s.read(&mut buf)).await {
        Ok(Ok(0)) | Ok(Err(_)) => true,
        Ok(Ok(_)) => false,
        Err(_) => false,
    }
}

#[tokio::test]
async fn first_byte_dispatches_plain_http_versus_tls() {
    let (mut l, addr) = listener_with(memory_config(), Limits::default()).await;
    // Every plausible first byte of an HTTP/1.x request line is plain; the TLS handshake record
    // type (0x16) is TLS.
    for first in *b"GPHDO" {
        let mut c = TcpStream::connect(addr).await.unwrap();
        c.write_all(&[first]).await.unwrap();
        let (_s, meta) = accept(&mut l).await;
        assert!(
            !meta.tls && meta.cert.is_none(),
            "byte {first:#x} must be plain"
        );
        assert!(matches!(_s, ConnStream::Plain(_)));
    }
    let client = tokio::spawn(async move { handshake_any(addr, None, &["http/1.1"]).await });
    let (s, meta) = accept(&mut l).await;
    assert!(meta.tls);
    assert_eq!(meta.cert, Some(Selected::Fallback));
    assert!(matches!(s, ConnStream::Tls(_)));
    let (_, presented) = client.await.unwrap().unwrap();
    assert_eq!(presented.alpn.as_deref(), Some(&b"http/1.1"[..]));
}

/// Every accepted connection, plain or TLS, has `TCP_NODELAY` set through `set_accepted_nodelay`;
/// this test covers the `SniffListener` path.
#[tokio::test]
async fn an_accepted_connection_has_tcp_nodelay_set_plain_and_tls() {
    let (mut l, addr) = listener_with(memory_config(), Limits::default()).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"G").await.unwrap();
    let (plain, _) = accept(&mut l).await;
    let ConnStream::Plain(plain) = plain else {
        panic!("a plain first byte was not served as plain");
    };
    assert!(plain.nodelay().unwrap(), "a plain connection has Nagle on");
    let client = tokio::spawn(async move { handshake_any(addr, None, &["http/1.1"]).await });
    let (tls, _) = accept(&mut l).await;
    let ConnStream::Tls(tls) = tls else {
        panic!("a TLS first byte was not served as TLS");
    };
    assert!(
        tls.get_ref().0.nodelay().unwrap(),
        "a TLS connection has Nagle on"
    );
    client.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_ip_literal_or_sni_less_connection_gets_the_self_signed_fallback_carrying_the_ip_san() {
    let (mut l, addr, rt) = listener_with_runtime(memory_config(), Limits::default()).await;
    let fallback_fp = rt.admin_status().fallback_fingerprint.unwrap();
    for sni in [None, Some("localhost"), Some("uncovered.example")] {
        let fp = accept_fingerprint(&mut l, addr, sni).await;
        assert_eq!(fp, fallback_fp, "sni {sni:?}");
    }
    let client = tokio::spawn(async move { handshake_any(addr, None, &["h2"]).await });
    let (_s, _) = accept(&mut l).await;
    let (_, presented) = client.await.unwrap().unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(&presented.leaf).unwrap();
    let ips: Vec<_> = cert
        .subject_alternative_name()
        .unwrap()
        .unwrap()
        .value
        .general_names
        .iter()
        .filter_map(|n| match n {
            x509_parser::extensions::GeneralName::IPAddress(b) => Some(b.to_vec()),
            _ => None,
        })
        .collect();
    assert!(
        ips.contains(&vec![127, 0, 0, 1]),
        "loopback IP SAN present: {ips:?}"
    );
}

/// Handshakes with `sni`, returns the presented leaf's fingerprint.
async fn accept_fingerprint(l: &mut SniffListener, addr: SocketAddr, sni: Option<&str>) -> String {
    let sni = sni.map(str::to_string);
    let client = tokio::spawn(async move { handshake_any(addr, sni.as_deref(), &["h2"]).await });
    let (_s, meta) = accept(l).await;
    assert_eq!(meta.cert, Some(Selected::Fallback));
    let (_, presented) = client.await.unwrap().unwrap();
    sha256_hex(&presented.leaf)
}

#[tokio::test]
async fn an_idle_peer_is_dropped_at_the_deadline() {
    let limits = Limits {
        deadline: Duration::from_millis(300),
        ..Limits::default()
    };
    let (l, addr) = listener_with(memory_config(), limits).await;
    let mut idle = TcpStream::connect(addr).await.unwrap();
    assert!(closes_within(&mut idle, Duration::from_secs(5)).await);
    assert_eq!(
        l.in_flight(),
        0,
        "the permit is released with the connection"
    );
}

#[tokio::test]
async fn a_peer_that_starts_a_tls_handshake_and_stalls_is_dropped_at_the_deadline() {
    let limits = Limits {
        deadline: Duration::from_millis(300),
        ..Limits::default()
    };
    let (l, addr) = listener_with(memory_config(), limits).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    // A TLS record header announcing a handshake record longer than what follows.
    c.write_all(&[0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00])
        .await
        .unwrap();
    assert!(closes_within(&mut c, Duration::from_secs(5)).await);
    assert_eq!(l.in_flight(), 0);
}

#[tokio::test]
async fn tls_off_drops_a_tls_peer_and_still_serves_plain_http() {
    let cfg = Config {
        tls: TlsMode::Off,
        ..memory_config()
    };
    let (mut l, addr) = listener_with(cfg, Limits::default()).await;
    let hs = tokio::spawn(async move { handshake_any(addr, None, &["h2"]).await });
    assert!(
        hs.await.unwrap().is_err(),
        "a TLS handshake against a TLS-off port fails"
    );
    let mut plain = TcpStream::connect(addr).await.unwrap();
    plain.write_all(b"G").await.unwrap();
    let (_s, meta) = accept(&mut l).await;
    assert!(!meta.tls);
}

#[tokio::test]
async fn global_cap_refuses_extra_connections_and_the_permit_releases_at_hand_off() {
    let limits = Limits {
        global_cap: 2,
        per_ip_cap: 100,
        deadline: Duration::from_secs(30),
    };
    let (mut l, addr) = listener_with(memory_config(), limits).await;
    let mut a = TcpStream::connect(addr).await.unwrap();
    let b = TcpStream::connect(addr).await.unwrap();
    // Wait until both are admitted, then a third is refused immediately.
    wait_until(|| l.in_flight() == 2).await;
    let mut c = TcpStream::connect(addr).await.unwrap();
    assert!(
        closes_within(&mut c, Duration::from_secs(5)).await,
        "third is over the cap"
    );
    // Hand-off releases the permit: send a byte on `a`, accept it, in_flight drops to 1 while the
    // accepted stream is still open.
    a.write_all(b"G").await.unwrap();
    let (held, _) = accept(&mut l).await;
    wait_until(|| l.in_flight() == 1).await;
    // A slot is free again.
    let mut d = TcpStream::connect(addr).await.unwrap();
    d.write_all(b"G").await.unwrap();
    let (_d, _) = accept(&mut l).await;
    drop((held, b));
}

/// A listener over a source reporting `peers` as the peer of each accepted connection, in order.
async fn listener_with_peers(
    config: Config,
    limits: Limits,
    peers: Vec<SocketAddr>,
) -> (SniffListener, SocketAddr) {
    shadowcat::tls::install_default_provider();
    let (tcp, addr) = bind().await;
    let config = Arc::new(config);
    let rt = TlsRuntime::start(&config);
    let source = SyntheticPeers { inner: tcp, peers };
    (SniffListener::new(source, rt, config, limits), addr)
}

/// A peer address for a scripted source.
fn peer_at(ip: &str) -> SocketAddr {
    format!("{ip}:4000").parse().unwrap()
}

#[tokio::test]
async fn per_ip_cap_applies_to_ordinary_peers_and_not_to_trusted_proxies_or_loopback() {
    let limits = Limits {
        global_cap: 100,
        per_ip_cap: 1,
        deadline: Duration::from_secs(30),
    };
    // Ordinary peer: the second idle connection from the same IP is refused.
    let (l, addr) = listener_with_peers(
        memory_config(),
        limits,
        vec![peer_at("192.0.2.1"), peer_at("192.0.2.1")],
    )
    .await;
    let _first = TcpStream::connect(addr).await.unwrap();
    wait_until(|| l.in_flight() == 1).await;
    let mut second = TcpStream::connect(addr).await.unwrap();
    assert!(closes_within(&mut second, Duration::from_secs(5)).await);

    // The same peer listed in trusted_proxies is exempt from the per-IP cap.
    let trusted = Config {
        trusted_proxies: vec!["192.0.2.1".parse().unwrap()],
        ..memory_config()
    };
    let (l2, addr2) = listener_with_peers(trusted, limits, vec![peer_at("192.0.2.1"); 3]).await;
    let _a = TcpStream::connect(addr2).await.unwrap();
    let _b = TcpStream::connect(addr2).await.unwrap();
    let _c = TcpStream::connect(addr2).await.unwrap();
    wait_until(|| l2.in_flight() == 3).await;

    // A loopback peer is exempt without being listed.
    let (l3, addr3) = listener_with(memory_config(), limits).await;
    let _x = TcpStream::connect(addr3).await.unwrap();
    let _y = TcpStream::connect(addr3).await.unwrap();
    let _z = TcpStream::connect(addr3).await.unwrap();
    wait_until(|| l3.in_flight() == 3).await;
}

/// The exemption decided at accept time travels with the connection, for plain and TLS peers, so
/// the serve loop's live-connection caps apply the same rule as the admission caps.
#[tokio::test]
async fn the_cap_exemption_travels_with_the_connection_for_proxies_and_loopback_but_not_ordinary_peers(
) {
    let cases = [
        ("a listed proxy", "192.0.2.50", vec!["192.0.2.50"], true),
        ("an ordinary peer", "192.0.2.1", vec![], false),
        ("loopback", "127.0.0.1", vec![], true),
        ("a mapped loopback", "::ffff:127.0.0.1", vec![], true),
    ];
    for (name, ip, listed, expected) in cases {
        let ip = if ip.contains(':') {
            format!("[{ip}]")
        } else {
            ip.to_string()
        };
        let config = Config {
            trusted_proxies: listed.iter().map(|p: &&str| p.parse().unwrap()).collect(),
            ..memory_config()
        };
        let (mut l, addr) =
            listener_with_peers(config, Limits::default(), vec![peer_at(&ip); 2]).await;
        let mut plain = TcpStream::connect(addr).await.unwrap();
        plain.write_all(b"G").await.unwrap();
        let (_s, meta) = accept(&mut l).await;
        assert_eq!(meta.cap_exempt, expected, "{name}: plain connection");

        let client = tokio::spawn(async move { handshake_any(addr, None, &["http/1.1"]).await });
        let (_s, meta) = accept(&mut l).await;
        assert!(meta.tls);
        assert_eq!(meta.cap_exempt, expected, "{name}: TLS connection");
        let _ = client.await;
    }
}

async fn wait_until(mut cond: impl FnMut() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not reached");
}

/// A TCP source that fails a scripted number of times before delegating, recording when each
/// `accept` call started.
struct FlakyAccept {
    inner: TcpListener,
    errors: Vec<io::ErrorKind>,
    calls: Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
}

impl TcpAccept for FlakyAccept {
    async fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        self.calls.lock().unwrap().push(std::time::Instant::now());
        if let Some(kind) = self.errors.pop() {
            return Err(io::Error::from(kind));
        }
        self.inner.accept().await
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

#[tokio::test]
async fn accept_errors_never_end_the_loop_and_a_resource_error_pauses_it() {
    shadowcat::tls::install_default_provider();
    let (tcp, addr) = bind().await;
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    // Popped from the end: a connection-level error (ignored at once), then a resource error
    // (logged, one-second pause), then success.
    let source = FlakyAccept {
        inner: tcp,
        errors: vec![io::ErrorKind::Other, io::ErrorKind::ConnectionAborted],
        calls: calls.clone(),
    };
    let config = Arc::new(memory_config());
    let mut l = SniffListener::new(source, TlsRuntime::disabled(), config, Limits::default());
    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"G").await.unwrap();
    let (_s, meta) = accept(&mut l).await;
    assert!(!meta.tls);
    let calls = calls.lock().unwrap();
    assert!(calls.len() >= 3, "aborted, other, then the real accept");
    assert!(
        calls[1] - calls[0] < Duration::from_millis(500),
        "a connection-level error must be retried immediately"
    );
    assert!(
        calls[2] - calls[1] >= Duration::from_millis(900),
        "a resource error must pause the loop, not spin"
    );
}

/// A TCP source that reports scripted peer addresses (in accept order) for real connections.
struct SyntheticPeers {
    inner: TcpListener,
    peers: Vec<SocketAddr>,
}

impl TcpAccept for SyntheticPeers {
    async fn accept(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        let (sock, _real) = self.inner.accept().await?;
        Ok((sock, self.peers.remove(0)))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

#[tokio::test]
async fn per_peer_caps_key_ipv6_by_slash_64_and_canonicalise_mapped_peers_end_to_end() {
    shadowcat::tls::install_default_provider();
    let (tcp, addr) = bind().await;
    let peer = |s: &str| -> SocketAddr { format!("{s}:4000").parse().unwrap() };
    let script: Vec<(SocketAddr, bool)> = vec![
        (peer("[2001:db8:1:2::1]"), true),
        (peer("[2001:db8:1:2::2]"), false), // same /64 as the first
        (peer("[2001:db8:1:3::1]"), true),  // a neighbouring /64
        (peer("[::ffff:198.51.100.7]"), true),
        (peer("198.51.100.7"), false), // the same host, unmapped
        (peer("[::ffff:198.51.100.99]"), true), // trusted proxy, written mapped
        (peer("198.51.100.99"), true), // trusted proxy again: exempt from the per-peer cap
    ];
    let source = SyntheticPeers {
        inner: tcp,
        peers: script.iter().map(|(p, _)| *p).collect(),
    };
    let config = Arc::new(Config {
        trusted_proxies: vec!["198.51.100.99".parse().unwrap()],
        ..memory_config()
    });
    let limits = Limits {
        global_cap: 100,
        per_ip_cap: 1,
        deadline: Duration::from_secs(30),
    };
    let l = SniffListener::new(source, TlsRuntime::disabled(), config, limits);
    let mut held = Vec::new();
    let mut admitted = 0;
    for (peer, expect_admitted) in script {
        let mut c = TcpStream::connect(addr).await.unwrap();
        if expect_admitted {
            admitted += 1;
            wait_until(|| l.in_flight() == admitted).await;
        } else {
            assert!(
                closes_within(&mut c, Duration::from_secs(5)).await,
                "{peer} must be refused by the per-peer cap"
            );
            assert_eq!(l.in_flight(), admitted, "{peer} took no slot");
        }
        held.push(c);
    }
}

#[tokio::test]
async fn conn_meta_is_debug_and_clone() {
    let m = ConnMeta::tls("127.0.0.1:9".parse().unwrap(), Selected::Primary);
    let dbg = format!("{:?}", m.clone());
    assert!(dbg.contains("ConnMeta") && dbg.contains("Primary"));
}

#[tokio::test]
async fn the_port_is_released_when_the_listener_is_dropped() {
    let (l, addr) = listener_with(memory_config(), Limits::default()).await;
    drop(l);
    // The acceptor task is aborted on drop; the OS then refuses new connections.
    for _ in 0..80 {
        if TcpStream::connect(addr).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("port still accepting after the listener was dropped");
}

#[tokio::test]
async fn graceful_shutdown_releases_the_port_with_a_live_tls_connection() {
    // Serve the real router over the sniffing listener, hold a TLS connection open, trigger
    // shutdown, and prove `serve` returns and the port is refused afterwards.
    shadowcat::tls::install_default_provider();
    let (tcp, addr) = bind().await;
    let config = Arc::new(memory_config());
    let tls = TlsRuntime::start(&config);
    let l = SniffListener::new(tcp, tls.clone(), config.clone(), Limits::default());
    let repo = Arc::new(
        shadowcat::data::sqlite::SqliteRepository::connect("sqlite::memory:")
            .await
            .unwrap(),
    );
    let state = shadowcat::http::AppState {
        repo,
        config,
        setup_token: None,
        initialized: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ws: shadowcat::ws::WsState::new(),
        upload_rate: Arc::new(shadowcat::http::assets::UploadRateLimiter::new()),
        uploads: Arc::new(shadowcat::http::assets::uploads::UploadSessions::new()),
        auth_throttle: Arc::new(shadowcat::http::throttle::AuthThrottle::new()),
        write_barrier: Arc::new(tokio::sync::RwLock::new(())),
        preview_fetch_locks: Arc::new(dashmap::DashMap::new()),
        file_permits: Arc::new(shadowcat::http::assets::permits::StreamedFilePermits::production()),
        tls,
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        shadowcat::http::serve(l, state, async {
            let _ = rx.await;
        })
        .await
    });
    // A live TLS connection that has completed a request/response on keep-alive.
    let (mut stream, _) = handshake_any(addr, None, &["http/1.1"]).await.unwrap();
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = [0u8; 256];
    let n = timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&buf[..n])
        .unwrap()
        .starts_with("HTTP/1.1 200"));

    tx.send(()).unwrap();
    // The port is released while the TLS connection is still held open by this test.
    let mut released = false;
    for _ in 0..80 {
        if TcpStream::connect(addr).await.is_err() {
            released = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(released, "port still accepting after shutdown began");
    let _still_open = &stream;
    timeout(Duration::from_secs(15), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stream);
}

#[test]
fn san_sources_detect_never_panics_and_excludes_loopback() {
    let s = SanSources::detect(false);
    assert!(s.interface_ips.iter().all(|ip| !ip.is_loopback()));
}
