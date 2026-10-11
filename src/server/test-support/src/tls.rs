//! TLS client helpers.
//!
//! TLS client helpers for the integration suites: rustls clients under the explicit ring provider
//! that either trust exactly the server's own certificate (so the SAN, validity and
//! key-usage checks run for real) or accept any certificate (to READ whatever the server presents).

use std::net::SocketAddr;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// The explicit ring provider, mirroring `shadowcat::tls::ring_provider`.
fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    shadowcat::tls::ring_provider()
}

/// A client configuration that trusts exactly the given certificates as roots, so a handshake
/// succeeds only when the server presents one of them AND it is valid for the connected name.
pub fn trusting_config(trust: &[CertificateDer<'static>], alpn: &[&str]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for c in trust {
        roots.add(c.clone()).expect("trust anchor");
    }
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|p| p.as_bytes().to_vec()).collect();
    Arc::new(cfg)
}

/// A certificate verifier that accepts everything: for tests that only READ what the server
/// presents. Never used where trust is the subject.
#[derive(Debug)]
struct AcceptAny(Vec<SignatureScheme>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.clone()
    }
}

/// A client configuration that accepts any server certificate.
pub fn accept_any_config(alpn: &[&str]) -> Arc<ClientConfig> {
    let schemes = provider()
        .signature_verification_algorithms
        .supported_schemes();
    let mut cfg = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(schemes)))
        .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|p| p.as_bytes().to_vec()).collect();
    Arc::new(cfg)
}

/// What a completed handshake exposed.
#[derive(Debug, Clone)]
pub struct Presented {
    /// DER of the leaf certificate the server presented.
    pub leaf: Vec<u8>,
    /// The negotiated ALPN protocol, when any.
    pub alpn: Option<Vec<u8>>,
}

/// Upper bound on connect plus handshake in [`handshake_any`].
const HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Opens a TCP connection to `addr` and completes a TLS handshake presenting `sni` (`None` sends
/// no SNI, exactly like a browser addressing an IP literal), accepting any certificate.
pub async fn handshake_any(
    addr: SocketAddr,
    sni: Option<&str>,
    alpn: &[&str],
) -> std::io::Result<(TlsStream<TcpStream>, Presented)> {
    let name = match sni {
        Some(host) => ServerName::try_from(host.to_string()).expect("valid server name"),
        None => ServerName::IpAddress(addr.ip().into()),
    };
    // A handshake that never completes must fail the caller, not hang the test run.
    let stream = tokio::time::timeout(HANDSHAKE_DEADLINE, async {
        let sock = TcpStream::connect(addr).await?;
        TlsConnector::from(accept_any_config(alpn))
            .connect(name, sock)
            .await
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS handshake deadline"))??;
    let (_, conn) = stream.get_ref();
    let leaf = conn
        .peer_certificates()
        .and_then(|c| c.first())
        .map(|c| c.as_ref().to_vec())
        .unwrap_or_default();
    let alpn = conn.alpn_protocol().map(<[u8]>::to_vec);
    Ok((stream, Presented { leaf, alpn }))
}

/// Lowercase hex SHA-256 of `der`.
pub fn sha256_hex(der: &[u8]) -> String {
    use std::fmt::Write;
    ring::digest::digest(&ring::digest::SHA256, der)
        .as_ref()
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A byte pipe over one HTTP/2 stream (an extended-CONNECT tunnel), usable as the transport of a
/// WebSocket (RFC 8441).
pub struct H2Pipe {
    /// Server-to-client bytes.
    recv: h2::RecvStream,
    /// Client-to-server bytes.
    send: h2::SendStream<bytes::Bytes>,
    /// Bytes received but not yet consumed by the reader.
    pending: bytes::Bytes,
}

impl H2Pipe {
    /// Wraps the two halves of a tunnel stream.
    pub fn new(recv: h2::RecvStream, send: h2::SendStream<bytes::Bytes>) -> Self {
        Self {
            recv,
            send,
            pending: bytes::Bytes::new(),
        }
    }
}

impl tokio::io::AsyncRead for H2Pipe {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;
        if self.pending.is_empty() {
            match self.recv.poll_data(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let _ = self.recv.flow_control().release_capacity(chunk.len());
                    self.pending = chunk;
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(std::io::Error::other(e)));
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = self.pending.len().min(buf.remaining());
        let chunk = self.pending.split_to(n);
        buf.put_slice(&chunk);
        Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncWrite for H2Pipe {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        data: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        use std::task::Poll;
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        self.send.reserve_capacity(data.len());
        match self.send.poll_capacity(cx) {
            Poll::Ready(Some(Ok(cap))) => {
                let n = cap.min(data.len());
                match self
                    .send
                    .send_data(bytes::Bytes::copy_from_slice(&data[..n]), false)
                {
                    Ok(()) => Poll::Ready(Ok(n)),
                    Err(e) => Poll::Ready(Err(std::io::Error::other(e))),
                }
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(std::io::Error::other(e))),
            Poll::Ready(None) => Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let _ = self.send.send_data(bytes::Bytes::new(), true);
        std::task::Poll::Ready(Ok(()))
    }
}

/// An open HTTP/2 client connection to a server, ready to open tunnels.
pub struct H2Client {
    /// Request sender.
    pub sender: h2::client::SendRequest<bytes::Bytes>,
    /// The connection driver task; finished once the connection has ended.
    pub driver: tokio::task::JoinHandle<()>,
}

/// Connects to `addr` over TLS negotiating `h2` (any certificate accepted), spawns the h2
/// connection driver, and returns the request sender together with the negotiated ALPN.
pub async fn h2_connect(addr: SocketAddr) -> std::io::Result<(H2Client, Option<Vec<u8>>)> {
    let (stream, presented) = handshake_any(addr, None, &["h2"]).await?;
    let (sender, conn) = h2::client::handshake(stream)
        .await
        .map_err(std::io::Error::other)?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok((H2Client { sender, driver }, presented.alpn))
}

impl H2Client {
    /// Opens an RFC 8441 extended-CONNECT WebSocket tunnel to `path_and_query` carrying `cookie`
    /// and an `Accept-Encoding` of `gzip, br`, returning the response status and the tunnel pipe.
    pub async fn websocket(
        &mut self,
        authority: &str,
        path_and_query: &str,
        cookie: &str,
    ) -> std::io::Result<(http::StatusCode, H2Pipe)> {
        self.sender = self
            .sender
            .clone()
            .ready()
            .await
            .map_err(std::io::Error::other)?;
        let req = http::Request::builder()
            .method(http::Method::CONNECT)
            .uri(format!("https://{authority}{path_and_query}"))
            .header("sec-websocket-version", "13")
            .header("accept-encoding", "gzip, br")
            .header("cookie", cookie)
            .extension(h2::ext::Protocol::from_static("websocket"))
            .body(())
            .map_err(std::io::Error::other)?;
        let (response, send) = self
            .sender
            .send_request(req, false)
            .map_err(std::io::Error::other)?;
        let response = response.await.map_err(std::io::Error::other)?;
        let status = response.status();
        let recv = response.into_body();
        Ok((status, H2Pipe::new(recv, send)))
    }
}
