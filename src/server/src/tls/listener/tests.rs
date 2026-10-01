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
    let Sniffed::Hello(hello) = sniff_and_hello(server_io, peer, Some(cell_ref)).await else {
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
    let out = negotiate_with(server_io, peer, Some(&cell)).await;
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
    let served = negotiate_with(server_io, peer, Some(cell.as_ref())).await;
    client
        .await
        .unwrap()
        .expect("the client's handshake trusts the post-swap certificate");
    assert!(served.is_some());
}
