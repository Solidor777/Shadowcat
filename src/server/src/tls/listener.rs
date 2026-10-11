//! The sniffing listener: ONE port serves plain HTTP and TLS. The first byte of each connection
//! decides (`0x16`, the TLS handshake record type, means TLS; anything else is plain HTTP), so
//! `http://host:port` and `https://host:port` both serve the full app and nothing is redirected.
//!
//! Accept path guarantees:
//! - one 10 s deadline bounds sniffing AND the whole TLS handshake (a peer that stalls
//!   mid-handshake is dropped like one that sends nothing). There is deliberately no shorter
//!   first-byte deadline: browsers open speculative connections (`rel=preconnect`, omnibox
//!   prediction) that carry no bytes until a request needs them and close any that goes unused
//!   for about 10 s (web.dev, "Establish network connections early to improve perceived page
//!   speed", caution note on `rel=preconnect`), so a silent socket up to that age is ordinary
//!   browser behaviour and the same 10 s is the longest silence worth holding. A shorter deadline
//!   would drop connections a browser is about to use (it reconnects at the cost of a round
//!   trip); a longer one only lets a silent peer hold a cap slot longer. Not verified: real
//!   omnibox preconnect against this listener (Playwright's Chromium and Firefox, given a
//!   preconnect link, opened no zero-byte connection within 25 s);
//! - admission caps: a global in-flight cap and a per-peer cap keyed by [`peer_key`](crate::http::peer_slots::peer_key) (an IPv4
//!   address, or the /64 prefix of an IPv6 address: a single IPv6 subscriber holds a whole /64, so
//!   a per-address key would let one host mint 2^64 keys; IPv4-mapped IPv6 peers count as their
//!   IPv4 address). Peers for which `Config::is_cap_exempt` holds (a listed trusted proxy or a
//!   loopback peer) are exempt from the per-peer cap because every player behind such a peer shares
//!   its address; they remain bound by the global cap. The caps bound the pre-hand-off phase, where a connection is unauthenticated and costs
//!   the peer nothing; afterwards `crate::http::conn_serve::ConnLimits` bounds header-read, HTTP/2
//!   idle time and the live-connection count (global and per peer, the same keying and
//!   exemption, carried to it in `ConnMeta::cap_exempt`), and request-level throttles bound
//!   abuse;
//! - accept errors follow axum's own policy ([`crate::http::conn_serve::accept_error_pause`]):
//!   connection-level errors are ignored, others are logged and followed by a 1 s pause, and the
//!   accept loop never exits on error;
//! - dropping the listener (graceful shutdown) releases the port immediately.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;

use super::resolver::{finish_phase, hello_phase, CertResolver};
use super::{ConnMeta, Selected, TlsRuntime};
use crate::config::Config;
use crate::http::conn_serve::{accept_error_pause, set_accepted_nodelay, ConnSource, ServeIo};
use crate::http::peer_slots::PeerSlots;

/// Capacity of the hand-off queue between the acceptor task and [`SniffListener::accept`]. A
/// connection waiting in it holds a descriptor and belongs to neither the pre-hand-off cap (its
/// permit is released once the send completes) nor the live cap (the serve loop admits it later),
/// so `crate::fd_budget` subtracts this from the descriptor budget.
pub const HANDOFF_QUEUE_CAP: usize = 128;

/// The TLS handshake record type: the first byte of every TLS connection's ClientHello record.
const TLS_HANDSHAKE_RECORD: u8 = 0x16;

/// A source of raw TCP connections. Implemented for `tokio::net::TcpListener`; the seam exists so
/// accept-error handling can be exercised with a source that fails.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::listener::TcpAccept;
///
/// # #[tokio::main] async fn main() {
/// let mut l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// assert!(TcpAccept::local_addr(&l).unwrap().port() != 0);
/// # }
/// ```
pub trait TcpAccept: Send + 'static {
    /// Accepts one connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::listener::TcpAccept;
    ///
    /// # #[tokio::main] async fn main() {
    /// let mut l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// let addr = TcpAccept::local_addr(&l).unwrap();
    /// let _client = tokio::net::TcpStream::connect(addr).await.unwrap();
    /// let (_sock, peer) = TcpAccept::accept(&mut l).await.unwrap();
    /// assert!(peer.ip().is_loopback());
    /// # }
    /// ```
    fn accept(&mut self) -> impl Future<Output = io::Result<(TcpStream, SocketAddr)>> + Send;
    /// The bound address.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::listener::TcpAccept;
    ///
    /// # #[tokio::main] async fn main() {
    /// let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// assert!(TcpAccept::local_addr(&l).unwrap().ip().is_loopback());
    /// # }
    /// ```
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

impl TcpAccept for TcpListener {
    fn accept(&mut self) -> impl Future<Output = io::Result<(TcpStream, SocketAddr)>> + Send {
        TcpListener::accept(self)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        TcpListener::local_addr(self)
    }
}

/// Admission and deadline limits of a [`SniffListener`].
///
/// # Examples
///
/// ```
/// let l = shadowcat::tls::listener::Limits::default();
/// assert_eq!((l.global_cap, l.per_ip_cap), (1024, 32));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum connections in flight before hand-off, across all peers.
    pub global_cap: usize,
    /// Maximum connections in flight before hand-off from one [`peer_key`](crate::http::peer_slots::peer_key).
    pub per_ip_cap: u32,
    /// Bound on sniffing plus the whole TLS handshake.
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            global_cap: 1024,
            per_ip_cap: 32,
            deadline: Duration::from_secs(10),
        }
    }
}

/// The IO type the listener hands to hyper: a plain socket or a server-side TLS stream.
///
/// # Examples
///
/// ```
/// fn assert_io<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static>() {}
/// assert_io::<shadowcat::tls::listener::ConnStream>();
/// ```
pub enum ConnStream {
    /// An unencrypted connection.
    Plain(TcpStream),
    /// A TLS connection terminated by this process.
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for ConnStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ConnStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ConnStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ConnStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ConnStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ConnStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ConnStream::Plain(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            ConnStream::Tls(s) => Pin::new(s.as_mut()).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            ConnStream::Plain(s) => s.is_write_vectored(),
            ConnStream::Tls(s) => s.is_write_vectored(),
        }
    }
}

impl ConnStream {
    /// The ALPN protocol the TLS handshake negotiated; `None` for plain connections and for a TLS
    /// client that offered none.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(sock: tokio::net::TcpStream) {
    /// let plain = shadowcat::tls::listener::ConnStream::Plain(sock);
    /// assert_eq!(plain.alpn(), None);
    /// # }
    /// ```
    pub fn alpn(&self) -> Option<&[u8]> {
        match self {
            ConnStream::Plain(_) => None,
            ConnStream::Tls(s) => s.get_ref().1.alpn_protocol(),
        }
    }
}

impl ServeIo for ConnStream {
    fn is_h2(&self) -> bool {
        self.alpn() == Some(b"h2")
    }
}

/// The connection source `crate::http::serve` drives: every connection has finished sniffing and,
/// for TLS, its handshake.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use shadowcat::config::Config;
/// use shadowcat::tls::listener::{Limits, SniffListener};
/// use shadowcat::tls::TlsRuntime;
///
/// # #[tokio::main] async fn main() {
/// let config = Arc::new(Config::default());
/// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// let listener = SniffListener::new(tcp, TlsRuntime::start(&config), config, Limits::default());
/// assert!(listener.local_addr().unwrap().port() != 0);
/// # }
/// ```
pub struct SniffListener {
    /// Connections that finished sniffing / handshaking.
    rx: mpsc::Receiver<(ConnStream, ConnMeta)>,
    /// The bound address.
    local: SocketAddr,
    /// The acceptor task; aborted on drop so the port is released immediately.
    task: tokio::task::JoinHandle<()>,
    /// Admission counters, shared with the acceptor and connection tasks.
    admission: Arc<PeerSlots>,
}

impl SniffListener {
    /// Starts accepting from `source`. `runtime` supplies the swap cell (`None` resolver = TLS off,
    /// TLS-looking peers are dropped); `config` supplies the per-peer cap exemption (`Config::is_cap_exempt`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// use shadowcat::tls::listener::{Limits, SniffListener};
    /// use shadowcat::tls::TlsRuntime;
    ///
    /// # #[tokio::main] async fn main() {
    /// let config = Arc::new(Config { tls: shadowcat::config::TlsMode::Off, ..Default::default() });
    /// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// let l = SniffListener::new(tcp, TlsRuntime::start(&config), config, Limits::default());
    /// assert_eq!(l.in_flight(), 0);
    /// # }
    /// ```
    pub fn new<A: TcpAccept>(
        mut source: A,
        runtime: Arc<TlsRuntime>,
        config: Arc<Config>,
        limits: Limits,
    ) -> Self {
        let local = source
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        let (tx, rx) = mpsc::channel(HANDOFF_QUEUE_CAP);
        let admission = Arc::new(
            PeerSlots::new(limits.global_cap, limits.per_ip_cap).named("pre-hand-off connections"),
        );
        let adm = admission.clone();
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    r = source.accept() => r,
                    _ = tx.closed() => break,
                };
                match accepted {
                    Ok((sock, peer)) => {
                        let exempt = config.is_cap_exempt(peer.ip());
                        let Some(permit) = adm.admit(peer.ip(), exempt) else {
                            continue;
                        };
                        set_accepted_nodelay(&sock, peer);
                        let runtime = runtime.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            if let Some((io, mut meta)) =
                                negotiate(sock, peer, &runtime, limits.deadline).await
                            {
                                // The one place the exemption is decided: the same value that
                                // skipped the admission per-peer cap travels with the connection
                                // so the live-connection caps apply the identical rule.
                                meta.cap_exempt = exempt;
                                let _ = tx.send((io, meta)).await;
                            }
                            drop(permit);
                        });
                    }
                    Err(e) => {
                        if let Some(pause) = accept_error_pause(&e) {
                            tokio::time::sleep(pause).await;
                        }
                    }
                }
            }
        });
        Self {
            rx,
            local,
            task,
            admission,
        }
    }

    /// The bound socket address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// use shadowcat::tls::listener::{Limits, SniffListener};
    /// use shadowcat::tls::TlsRuntime;
    ///
    /// # #[tokio::main] async fn main() {
    /// let config = Arc::new(Config::default());
    /// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// let l = SniffListener::new(tcp, TlsRuntime::disabled(), config, Limits::default());
    /// assert!(l.local_addr().unwrap().port() != 0);
    /// # }
    /// ```
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local)
    }

    /// Connections currently between accept and hand-off.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// use shadowcat::tls::listener::{Limits, SniffListener};
    /// use shadowcat::tls::TlsRuntime;
    ///
    /// # #[tokio::main] async fn main() {
    /// let config = Arc::new(Config::default());
    /// let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    /// let l = SniffListener::new(tcp, TlsRuntime::disabled(), config, Limits::default());
    /// assert_eq!(l.in_flight(), 0);
    /// # }
    /// ```
    pub fn in_flight(&self) -> usize {
        self.admission.held()
    }
}

impl Drop for SniffListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ConnSource for SniffListener {
    type Io = ConnStream;

    /// The next negotiated connection. `Ok(None)` only when the acceptor task has ended (it exits
    /// solely by panicking, since accept errors never end it), which the serve loop reports and
    /// stops on rather than idling forever with a dead acceptor. Never an `Err`: the acceptor
    /// task paces its own accept errors, and `Receiver::recv` is cancel-safe.
    async fn accept(&mut self) -> io::Result<Option<(ConnStream, ConnMeta)>> {
        Ok(self.rx.recv().await)
    }
}

/// Sniffs the first byte of `sock` and, for TLS, runs the two-phase handshake, all under one
/// deadline. `None` means the connection was dropped (idle, malformed, stalled, TLS while TLS is
/// off, or a TLS-ALPN-01 validation connection that is closed after its handshake).
async fn negotiate(
    sock: TcpStream,
    peer: SocketAddr,
    runtime: &TlsRuntime,
    deadline: Duration,
) -> Option<(ConnStream, ConnMeta)> {
    let by = Instant::now() + deadline;
    match tokio::time::timeout_at(by, negotiate_inner(sock, peer, runtime)).await {
        Ok(ready) => ready,
        Err(_) => {
            tracing::debug!(%peer, "connection dropped: no complete first request or handshake before the deadline");
            None
        }
    }
}

/// The deadline-free body of [`negotiate`].
async fn negotiate_inner(
    sock: TcpStream,
    peer: SocketAddr,
    runtime: &TlsRuntime,
) -> Option<(ConnStream, ConnMeta)> {
    negotiate_with(sock, peer, runtime.resolver().map(Arc::as_ref)).await
}

/// Sniff, hello and finish in sequence. The two stages are separate functions so a test can land
/// a hot swap between them; this is the only place they are chained.
async fn negotiate_with(
    sock: TcpStream,
    peer: SocketAddr,
    resolver: Option<&CertResolver>,
) -> Option<(ConnStream, ConnMeta)> {
    match sniff_and_hello(sock, peer, resolver).await {
        Sniffed::Ready(ready) => ready,
        Sniffed::Hello(hello) => finish(*hello).await,
    }
}

/// Outcome of [`sniff_and_hello`].
enum Sniffed {
    /// Nothing more to negotiate: a plain connection, or one dropped.
    Ready(Option<(ConnStream, ConnMeta)>),
    /// A TLS connection whose hello phase selected a certificate; [`finish`] completes it.
    Hello(Box<HelloDone>),
}

/// A TLS connection between the hello phase and the finish phase: the window in which a hot swap
/// can land. The certificate selection and the snapshot it came from travel together, so the
/// finish phase serves exactly what was selected.
struct HelloDone {
    /// The handshake, paused after the ClientHello.
    start: tokio_rustls::StartHandshake<TcpStream>,
    /// The snapshot the selection was made from.
    snapshot: Arc<super::resolver::Snapshot>,
    /// The certificate the hello phase chose.
    selected: Selected,
    /// The peer address, recorded in the connection metadata.
    peer: SocketAddr,
}

/// Sniffs the first byte and, for TLS, reads the ClientHello and selects a certificate. The
/// snapshot is taken from the resolver once, AFTER the ClientHello arrives (so a slow hello does
/// not pin a snapshot that predates the certificate a swap installed meanwhile), and the finish
/// phase uses the same one.
async fn sniff_and_hello(
    sock: TcpStream,
    peer: SocketAddr,
    resolver: Option<&CertResolver>,
) -> Sniffed {
    let mut first = [0u8; 1];
    match sock.peek(&mut first).await {
        Ok(1) => {}
        _ => return Sniffed::Ready(None),
    }
    if first[0] != TLS_HANDSHAKE_RECORD {
        return Sniffed::Ready(Some((ConnStream::Plain(sock), ConnMeta::plain(peer))));
    }
    let Some(resolver) = resolver else {
        return Sniffed::Ready(None);
    };
    let start = match LazyConfigAcceptor::new(rustls::server::Acceptor::default(), sock).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(%peer, error = %e, "TLS hello failed");
            return Sniffed::Ready(None);
        }
    };
    let snapshot = resolver.snapshot();
    let selected = hello_phase(&start, &snapshot);
    Sniffed::Hello(Box::new(HelloDone {
        start,
        snapshot,
        selected,
        peer,
    }))
}

/// Completes the handshake of a connection that passed [`sniff_and_hello`].
async fn finish(hello: HelloDone) -> Option<(ConnStream, ConnMeta)> {
    let HelloDone {
        start,
        snapshot,
        selected,
        peer,
    } = hello;
    match finish_phase(start, snapshot, selected, peer).await {
        Ok((_stream, _)) if selected == Selected::Challenge => None,
        Ok((stream, meta)) => Some((ConnStream::Tls(Box::new(stream)), meta)),
        Err(e) => {
            tracing::debug!(%peer, error = %e, "TLS handshake failed");
            None
        }
    }
}

#[cfg(test)]
mod tests;
