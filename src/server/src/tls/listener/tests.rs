//! The two-phase negotiation, without sockets beyond a loopback pair.

use super::*;

// ---------------------------------------------------------------------------------------------
// Two-phase negotiation
// ---------------------------------------------------------------------------------------------

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::RootCertStore;
use tokio::io::AsyncWriteExt;

use crate::tls::resolver::Snapshot;
use crate::tls::selfsigned::{generate, SanSet};

/// A ring-provider client that trusts exactly `anchor`, offering `alpn`.
fn client_for(anchor: &CertificateDer<'static>, alpn: &[&[u8]]) -> tokio_rustls::TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.add(anchor.clone()).unwrap();
    let mut cfg = rustls::ClientConfig::builder_with_provider(crate::tls::ring_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
}

/// A connected loopback pair: (client side, server side, client's address as the server sees it).
async fn pair() -> (TcpStream, TcpStream, SocketAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let client = TcpStream::connect(addr).await.unwrap();
    let (server, peer) = l.accept().await.unwrap();
    (client, server, peer)
}

#[tokio::test]
async fn a_hot_swap_between_hello_and_finish_cannot_split_the_decision() {
    // Between the hello phase (which selects) and the finish phase (which completes the
    // handshake) a NEW snapshot with no primary at all is installed. The certificate the client
    // reads and `ConnMeta.cert` must both still say "the primary the selection chose". The swap
    // is installed between the two stages the shipped negotiation chains (`sniff_and_hello`, then
    // `finish`), so no test hook exists in the production path.
    crate::tls::install_default_provider();
    let now = time::OffsetDateTime::now_utc();
    let primary = generate(
        &SanSet {
            dns: vec!["game.example".into()],
            ips: vec![],
        },
        now,
    )
    .unwrap();
    let fallback = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    let cell = CertResolver::new(
        Snapshot::new(
            Some(Arc::new(primary.loaded)),
            fallback.clone(),
            Default::default(),
        )
        .unwrap(),
    );
    let (client_io, server_io, peer) = pair().await;
    let connector = client_for(&primary.cert_der, &[b"http/1.1"]);
    let client = tokio::spawn(async move {
        let name = ServerName::try_from("game.example").unwrap();
        let mut s = connector.connect(name, client_io).await.unwrap();
        let leaf = s.get_ref().1.peer_certificates().unwrap()[0]
            .as_ref()
            .to_vec();
        let _ = s.shutdown().await;
        leaf
    });

    let fb = fallback.clone();
    let cell_ref = &cell;
    let Sniffed::Hello(hello) = sniff_and_hello(server_io, peer, false, Some(cell_ref)).await
    else {
        panic!("a TLS ClientHello reaches the hello stage");
    };
    cell_ref
        .update(|_| Snapshot::new(None, fb, Default::default()))
        .unwrap();
    let (stream, meta) = finish(*hello).await.expect("the handshake completes");
    assert!(matches!(stream, ConnStream::Tls(_)));
    assert_eq!(
        cell.snapshot().select(Some("game.example"), &[]),
        Selected::Fallback,
        "the injected swap landed"
    );
    assert_eq!(meta.cert, Some(Selected::Primary));
    let presented = client.await.unwrap();
    assert_eq!(presented, primary.cert_der.as_ref().to_vec());
}

#[tokio::test]
async fn a_tls_alpn_01_validation_connection_completes_its_handshake_and_is_dropped() {
    crate::tls::install_default_provider();
    let now = time::OffsetDateTime::now_utc();
    let challenge = generate(
        &SanSet {
            dns: vec!["game.example".into()],
            ips: vec![],
        },
        now,
    )
    .unwrap();
    let fallback = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    let mut challenges = std::collections::BTreeMap::new();
    challenges.insert(
        "game.example".to_string(),
        challenge.loaded.certified_key().clone(),
    );
    let cell = CertResolver::new(Snapshot::new(None, fallback, challenges).unwrap());
    let (client_io, server_io, peer) = pair().await;
    let connector = client_for(&challenge.cert_der, &[b"acme-tls/1"]);
    let client = tokio::spawn(async move {
        let name = ServerName::try_from("game.example").unwrap();
        let s = connector.connect(name, client_io).await.unwrap();
        s.get_ref().1.alpn_protocol().map(|p| p.to_vec())
    });
    let out = negotiate_with(server_io, peer, false, Some(&cell)).await;
    assert!(
        out.is_none(),
        "a validation connection is never handed to HTTP"
    );
    assert_eq!(
        client.await.unwrap().as_deref(),
        Some(&b"acme-tls/1"[..]),
        "the validator's handshake completed with the challenge certificate"
    );
}

#[tokio::test]
async fn the_snapshot_is_taken_after_the_client_hello_arrives() {
    // The client sends the first byte of its hello, waits, installs a NEW fallback certificate,
    // then sends the rest. The connection must be served with the certificate installed before
    // the hello COMPLETED: the client trusts only the new certificate, so a snapshot taken at
    // the first byte would fail its handshake.
    crate::tls::install_default_provider();
    let now = time::OffsetDateTime::now_utc();
    let old = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    let new = generate(&SanSet::default(), now).unwrap();
    let cell = Arc::new(CertResolver::new(
        Snapshot::new(None, old, Default::default()).unwrap(),
    ));
    let (client_io, server_io, peer) = pair().await;
    let mut roots = RootCertStore::empty();
    roots.add(new.cert_der.clone()).unwrap();
    let cfg = Arc::new(
        rustls::ClientConfig::builder_with_provider(crate::tls::ring_provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let swap_cell = cell.clone();
    let new_loaded = Arc::new(new.loaded);
    let client_std = client_io.into_std().unwrap();
    client_std.set_nonblocking(false).unwrap();
    let client = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut sock = client_std;
        let name = ServerName::try_from("localhost").unwrap();
        let mut conn = rustls::ClientConnection::new(cfg, name).unwrap();
        let mut hello = Vec::new();
        while conn.wants_write() {
            conn.write_tls(&mut hello).unwrap();
        }
        sock.write_all(&hello[..1]).unwrap();
        sock.flush().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        swap_cell
            .update(|cur| cur.with_fallback(new_loaded))
            .unwrap();
        sock.write_all(&hello[1..]).unwrap();
        sock.flush().unwrap();
        conn.complete_io(&mut sock).map(|_| ())
    });
    let served = negotiate_with(server_io, peer, false, Some(cell.as_ref())).await;
    client
        .await
        .unwrap()
        .expect("the client's handshake trusts the post-swap certificate");
    assert!(served.is_some());
}

// ---------------------------------------------------------------------------------------------
// Certificate scope by peer address
// ---------------------------------------------------------------------------------------------

use std::sync::Mutex;

use crate::tls::selfsigned::{RcgenGenerator, SanSources};

/// Interface addresses of the synthetic host: a private one, a public-looking global one and a
/// unique-local one. A public peer must read none of them.
const HOST_INTERFACES: [&str; 3] = ["192.168.1.20", "198.51.100.50", "fd00::5"];

/// A runtime with a persisted TLS directory and `HOST_INTERFACES` as the host's interfaces.
fn host_runtime(dir: &std::path::Path) -> Arc<TlsRuntime> {
    host_runtime_naming(dir, Vec::new())
}

/// [`host_runtime`] with `tls_hostnames` configured.
fn host_runtime_naming(dir: &std::path::Path, tls_hostnames: Vec<String>) -> Arc<TlsRuntime> {
    crate::tls::install_default_provider();
    TlsRuntime::start_with(
        &Config {
            db: "sqlite::memory:".into(),
            tls_dir: Some(dir.to_string_lossy().into_owned()),
            tls_hostnames,
            ..Config::default()
        },
        Arc::new(RcgenGenerator),
        SanSources {
            interface_ips: HOST_INTERFACES.iter().map(|s| s.parse().unwrap()).collect(),
            ..SanSources::default()
        },
        time::OffsetDateTime::now_utc(),
    )
}

/// Every SAN entry of a DER leaf, DNS names lowercase and addresses in canonical text.
fn sans_of(der: &[u8]) -> Vec<String> {
    use x509_parser::extensions::GeneralName;
    let (_, parsed) = x509_parser::parse_x509_certificate(der).unwrap();
    let ext = parsed.subject_alternative_name().unwrap().unwrap();
    ext.value
        .general_names
        .iter()
        .filter_map(|n| match n {
            GeneralName::DNSName(d) => Some(d.to_string()),
            GeneralName::IPAddress(b) => match b.len() {
                4 => Some(std::net::IpAddr::from(<[u8; 4]>::try_from(*b).unwrap()).to_string()),
                16 => Some(std::net::IpAddr::from(<[u8; 16]>::try_from(*b).unwrap()).to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The SAN entries of the certificate the negotiation presents to a peer at `peer`. The socket is
/// a loopback pair; `peer` is the address the acceptor would have recorded, the only input scope
/// selection reads.
async fn sans_shown_to(runtime: &TlsRuntime, peer: SocketAddr, trusted_proxy: bool) -> Vec<String> {
    let cell = runtime.resolver().expect("TLS is on");
    let snapshot = cell.snapshot();
    let mut roots = RootCertStore::empty();
    for cert in [
        snapshot.fallback(),
        snapshot.public_fallback(),
        snapshot.host_fallback(),
    ] {
        roots.add(cert.certified_key().cert[0].clone()).unwrap();
    }
    let cfg = rustls::ClientConfig::builder_with_provider(crate::tls::ring_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let (client_io, server_io, _) = pair().await;
    let client = tokio::spawn(async move {
        let name = ServerName::try_from("localhost").unwrap();
        let mut s = connector.connect(name, client_io).await.unwrap();
        let leaf = s.get_ref().1.peer_certificates().unwrap()[0]
            .as_ref()
            .to_vec();
        let _ = s.shutdown().await;
        leaf
    });
    let served = negotiate_with(server_io, peer, trusted_proxy, Some(cell)).await;
    assert!(served.is_some(), "the handshake completes");
    sans_of(&client.await.unwrap())
}

#[tokio::test]
async fn a_public_peer_never_reads_an_interface_address_and_a_lan_peer_does() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = host_runtime(dir.path());
    for public in ["203.0.113.9:4000", "[2001:db8::1]:4000", "100.64.0.7:4000"] {
        let sans = sans_shown_to(&runtime, public.parse().unwrap(), false).await;
        for interface in HOST_INTERFACES {
            assert!(
                !sans.contains(&interface.to_string()),
                "{public} read {interface}: {sans:?}"
            );
        }
        assert!(sans.contains(&"localhost".to_string()), "{sans:?}");
    }
    for lan in [
        "192.168.1.7:4000",
        "10.1.2.3:4000",
        "172.20.0.4:4000",
        "169.254.3.4:4000",
        "[fd12::9]:4000",
        "[fe80::9]:4000",
        "[::ffff:10.0.0.4]:4000",
    ] {
        let sans = sans_shown_to(&runtime, lan.parse().unwrap(), false).await;
        for interface in HOST_INTERFACES {
            assert!(
                sans.contains(&interface.to_string()),
                "{lan} should read {interface}: {sans:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_loopback_peer_reads_localhost_names_only_and_a_listed_proxy_the_public_certificate() {
    // The host machine's own browser connects from loopback and receives a certificate naming
    // only localhost, 127.0.0.1 and ::1: no interface address, no configured hostname. A listed
    // trusted proxy fronts unknown players and receives the public certificate, which carries the
    // configured hostnames but no interface address.
    let dir = tempfile::tempdir().unwrap();
    let runtime = host_runtime_naming(dir.path(), vec!["game.example".into()]);
    let config = Config {
        trusted_proxies: vec!["10.0.0.9".parse().unwrap(), "127.0.0.2".parse().unwrap()],
        ..Default::default()
    };
    for loopback in ["127.0.0.1:4000", "[::1]:4000", "[::ffff:127.0.0.1]:4000"] {
        let peer: SocketAddr = loopback.parse().unwrap();
        assert!(!config.is_trusted_proxy(peer.ip()), "{loopback}");
        let mut sans = sans_shown_to(&runtime, peer, config.is_trusted_proxy(peer.ip())).await;
        sans.sort();
        assert_eq!(sans, ["127.0.0.1", "::1", "localhost"], "{loopback}");
    }
    for proxy in ["10.0.0.9:4000", "127.0.0.2:4000"] {
        let peer: SocketAddr = proxy.parse().unwrap();
        assert!(config.is_trusted_proxy(peer.ip()), "{proxy}");
        let sans = sans_shown_to(&runtime, peer, config.is_trusted_proxy(peer.ip())).await;
        for interface in HOST_INTERFACES {
            assert!(
                !sans.contains(&interface.to_string()),
                "{proxy} read {interface}: {sans:?}"
            );
        }
        assert!(
            sans.contains(&"game.example".to_string()),
            "{proxy} is shown the public certificate: {sans:?}"
        );
    }
    let other: SocketAddr = "10.0.0.10:4000".parse().unwrap();
    assert!(!config.is_trusted_proxy(other.ip()));
    let sans = sans_shown_to(&runtime, other, config.is_trusted_proxy(other.ip())).await;
    assert!(
        HOST_INTERFACES
            .iter()
            .all(|i| sans.contains(&i.to_string())),
        "a private peer that is not a listed proxy keeps the LAN certificate: {sans:?}"
    );
}

/// The SAN entries of the leaf a real TLS client from loopback reads through a `SniffListener`
/// built over `runtime` and `config`.
async fn sans_through_a_sniff_listener(runtime: Arc<TlsRuntime>, config: Config) -> Vec<String> {
    let snapshot = runtime.resolver().expect("TLS is on").snapshot();
    let mut roots = RootCertStore::empty();
    for cert in [
        snapshot.fallback(),
        snapshot.public_fallback(),
        snapshot.host_fallback(),
    ] {
        roots.add(cert.certified_key().cert[0].clone()).unwrap();
    }
    let cfg = rustls::ClientConfig::builder_with_provider(crate::tls::ring_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = tcp.local_addr().unwrap();
    let mut listener = SniffListener::new(tcp, runtime, Arc::new(config), Limits::default());
    let client = tokio::spawn(async move {
        let io = TcpStream::connect(addr).await.unwrap();
        let name = ServerName::try_from("localhost").unwrap();
        let mut s = connector.connect(name, io).await.unwrap();
        let leaf = s.get_ref().1.peer_certificates().unwrap()[0]
            .as_ref()
            .to_vec();
        let _ = s.shutdown().await;
        leaf
    });
    let _served = listener.accept().await.unwrap();
    sans_of(&client.await.unwrap())
}

#[tokio::test]
async fn the_sniff_listener_acceptor_hands_the_trusted_proxy_decision_to_the_negotiation() {
    // A loopback client reaches the certificate choice only through the acceptor, which computes
    // the trusted-proxy flag and passes it on. Both outcomes are observable here: an unlisted
    // loopback peer is shown the host-machine certificate and a listed one the public
    // certificate, and neither lists an interface address. An acceptor that passed the wrong
    // flag would show the LAN certificate (interface addresses) or swap the two.
    let dir = tempfile::tempdir().unwrap();
    let runtime = host_runtime_naming(dir.path(), vec!["game.example".into()]);

    let mut unlisted = sans_through_a_sniff_listener(runtime.clone(), Config::default()).await;
    unlisted.sort();
    assert_eq!(unlisted, ["127.0.0.1", "::1", "localhost"]);

    let listed = sans_through_a_sniff_listener(
        runtime,
        Config {
            trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
            ..Default::default()
        },
    )
    .await;
    for interface in HOST_INTERFACES {
        assert!(
            !listed.contains(&interface.to_string()),
            "a loopback client read {interface}: {listed:?}"
        );
    }
    assert!(listed.contains(&"game.example".to_string()), "{listed:?}");
}

#[tokio::test]
async fn the_public_certificate_lists_the_external_address_once_the_runtime_knows_it() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = host_runtime(dir.path());
    let public_peer: SocketAddr = "203.0.113.9:4000".parse().unwrap();
    let lan_peer: SocketAddr = "192.168.1.7:4000".parse().unwrap();
    let external = Arc::new(Mutex::new(None::<std::net::IpAddr>));
    let source = external.clone();
    runtime.set_external_source(Arc::new(move || *source.lock().unwrap()));
    let now = time::OffsetDateTime::now_utc();

    runtime.maintenance_tick(now);
    assert!(!sans_shown_to(&runtime, public_peer, false)
        .await
        .contains(&"198.51.100.77".to_string()));

    *external.lock().unwrap() = Some("198.51.100.77".parse().unwrap());
    runtime.maintenance_tick(now);
    assert!(sans_shown_to(&runtime, public_peer, false)
        .await
        .contains(&"198.51.100.77".to_string()));
    assert!(
        !sans_shown_to(&runtime, lan_peer, false)
            .await
            .contains(&"198.51.100.77".to_string()),
        "a LAN peer is shown the interface certificate, which carries no external address"
    );
}

#[tokio::test]
async fn a_changed_external_address_replaces_the_public_certificate_and_drops_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = host_runtime(dir.path());
    let public_peer: SocketAddr = "203.0.113.9:4000".parse().unwrap();
    let external = Arc::new(Mutex::new(Some("198.51.100.77".parse().unwrap())));
    let source = external.clone();
    runtime.set_external_source(Arc::new(move || *source.lock().unwrap()));
    let now = time::OffsetDateTime::now_utc();
    runtime.maintenance_tick(now);
    let first = runtime
        .resolver()
        .unwrap()
        .snapshot()
        .public_fallback()
        .fingerprint()
        .to_string();

    // The same address again changes nothing.
    runtime.maintenance_tick(now);
    let again = runtime
        .resolver()
        .unwrap()
        .snapshot()
        .public_fallback()
        .fingerprint()
        .to_string();
    assert_eq!(first, again);

    *external.lock().unwrap() = Some("198.51.100.88".parse().unwrap());
    runtime.maintenance_tick(now);
    let sans = sans_shown_to(&runtime, public_peer, false).await;
    assert!(sans.contains(&"198.51.100.88".to_string()), "{sans:?}");
    assert!(!sans.contains(&"198.51.100.77".to_string()), "{sans:?}");

    // The address going unknown is not a reason to replace it.
    let current = runtime
        .resolver()
        .unwrap()
        .snapshot()
        .public_fallback()
        .fingerprint()
        .to_string();
    *external.lock().unwrap() = None;
    runtime.maintenance_tick(now);
    assert_eq!(
        runtime
            .resolver()
            .unwrap()
            .snapshot()
            .public_fallback()
            .fingerprint(),
        current
    );
}

#[tokio::test]
async fn the_hand_off_queue_has_the_capacity_the_descriptor_budget_subtracts() {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Arc::new(Config::default());
    let listener = SniffListener::new(tcp, TlsRuntime::disabled(), config, Limits::default());
    assert_eq!(listener.rx.max_capacity(), HANDOFF_QUEUE_CAP);
}
