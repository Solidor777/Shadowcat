//! This module provides a serve-loop source whose connections record the instant the server side
//! drops them.
//!
//! A test that needs to know WHEN the server ended a connection cannot learn it from the peer
//! without reading, and reading from a peer that has stopped reading is exactly what changes the
//! outcome; the server's own drop of the socket is observable without touching the peer.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use shadowcat::http::conn_serve::{set_accepted_nodelay, ConnSource, ServeIo};
use shadowcat::tls::ConnMeta;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::h2c::EndHandle;

/// The instants at which the server dropped its end of each connection, in drop order.
pub type ClosedLog = Arc<Mutex<Vec<Instant>>>;

/// What the server's writes to its connections' sockets did, counted at the socket exactly as the
/// serve loop's write counter sees them (it wraps the same stream, so it sees the same results).
#[derive(Debug, Default)]
pub struct WriteStats {
    /// Bytes the socket accepted over every connection: the sum of each `Ready(Ok(n))` write.
    pub accepted: AtomicU64,
    /// Write polls that returned `Pending`: the writer parked.
    pub parks: AtomicU64,
    /// Each write the socket accepted: the instant and the bytes, in order.
    grants: Mutex<Vec<(Instant, u64)>>,
}

impl WriteStats {
    /// Counts one accepted write of `bytes` at `at`: the total and the grant
    /// [`WriteStats::last_progress_at`] reads.
    pub fn note_accepted(&self, at: Instant, bytes: u64) {
        self.accepted.fetch_add(bytes, Ordering::Relaxed);
        if let Ok(mut grants) = self.grants.lock() {
            grants.push((at, bytes));
        }
    }

    /// The instant the socket last accepted progress at `floor` bytes: walking back from the newest
    /// write, the grant at which the bytes accepted since reach `floor`. Everything accepted after
    /// it totals under `floor`, so the connection unit's window (fewer than `floor` bytes accepted
    /// while the writer is parked) starts no later than this instant. `None` when the socket never
    /// accepted `floor` bytes.
    pub fn last_progress_at(&self, floor: u64) -> Option<Instant> {
        let grants = self.grants.lock().ok()?;
        let mut since = 0u64;
        grants.iter().rev().find_map(|(at, bytes)| {
            since += bytes;
            (since >= floor).then_some(*at)
        })
    }
}

/// A TCP stream that logs the instant it is dropped and reports whether it speaks HTTP/2.
pub struct ObservedStream {
    /// The socket.
    inner: TcpStream,
    /// Whether the connection claims ALPN `h2`.
    h2: bool,
    /// Where the drop instant goes.
    closed: ClosedLog,
    /// Where write outcomes are counted.
    stats: Arc<WriteStats>,
}

impl ObservedStream {
    /// Counts one write outcome.
    fn count(&self, result: &Poll<io::Result<usize>>) {
        match result {
            Poll::Pending => {
                self.stats.parks.fetch_add(1, Ordering::Relaxed);
            }
            Poll::Ready(Ok(n)) => {
                self.stats.note_accepted(Instant::now(), *n as u64);
            }
            Poll::Ready(Err(_)) => {}
        }
    }
}

impl Drop for ObservedStream {
    fn drop(&mut self) {
        if let Ok(mut log) = self.closed.lock() {
            log.push(Instant::now());
        }
    }
}

impl AsyncRead for ObservedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for ObservedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        self.count(&result);
        result
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        self.count(&result);
        result
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl ServeIo for ObservedStream {
    fn is_h2(&self) -> bool {
        self.h2
    }
}

/// A [`ConnSource`] over a real listener whose connections are [`ObservedStream`]s, and which
/// ends when its [`EndHandle`] fires.
pub struct ObservedSource {
    /// The bound listener.
    listener: TcpListener,
    /// Fires when the source should report that it has ended.
    end: oneshot::Receiver<()>,
    /// Whether every connection claims ALPN `h2`.
    h2: bool,
    /// Shared with every connection the source hands out.
    closed: ClosedLog,
    /// Shared with every connection the source hands out.
    stats: Arc<WriteStats>,
    /// Whether accepted sockets get `TCP_NODELAY` through `set_accepted_nodelay`, the function
    /// `impl ConnSource for TcpListener` and `SniffListener`'s accept task call.
    nodelay: bool,
}

impl ObservedSource {
    /// A source over `listener` whose connections speak HTTP/2 when `h2` and HTTP/1.1 otherwise,
    /// the handle that ends it, and the log of connection drops.
    pub fn new(listener: TcpListener, h2: bool) -> (Self, EndHandle, ClosedLog) {
        let (tx, end) = oneshot::channel();
        let closed = ClosedLog::default();
        (
            Self {
                listener,
                end,
                h2,
                closed: closed.clone(),
                stats: Arc::default(),
                nodelay: true,
            },
            EndHandle(tx),
            closed,
        )
    }
}

impl ObservedSource {
    /// Sets whether accepted sockets get `TCP_NODELAY` (on by default, as in production); the
    /// throughput ladder turns it off for its control.
    pub fn with_nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = nodelay;
        self
    }

    /// The write counts of every connection the source hands out.
    pub fn write_stats(&self) -> Arc<WriteStats> {
        self.stats.clone()
    }
}

impl ConnSource for ObservedSource {
    type Io = ObservedStream;

    async fn accept(&mut self) -> io::Result<Option<(ObservedStream, ConnMeta)>> {
        tokio::select! {
            accepted = self.listener.accept() => {
                let (sock, peer): (TcpStream, SocketAddr) = accepted?;
                if self.nodelay {
                    set_accepted_nodelay(&sock, peer);
                }
                Ok(Some((
                    ObservedStream {
                        inner: sock,
                        h2: self.h2,
                        closed: self.closed.clone(),
                        stats: self.stats.clone(),
                    },
                    ConnMeta::plain(peer),
                )))
            }
            _ = &mut self.end => Ok(None),
        }
    }
}
