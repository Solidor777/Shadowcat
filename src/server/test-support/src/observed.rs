//! A serve-loop source whose connections record the instant the server side drops them. A test that
//! needs to know WHEN the server ended a connection cannot learn it from the peer without reading,
//! and reading from a peer that has stopped reading is exactly what changes the outcome; the
//! server's own drop of the socket is observable without touching the peer.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use shadowcat::http::conn_serve::{ConnSource, ServeIo};
use shadowcat::tls::ConnMeta;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::h2c::EndHandle;

/// The instants at which the server dropped its end of each connection, in drop order.
pub type ClosedLog = Arc<Mutex<Vec<Instant>>>;

/// A TCP stream that logs the instant it is dropped and reports whether it speaks HTTP/2.
pub struct ObservedStream {
    /// The socket.
    inner: TcpStream,
    /// Whether the connection claims ALPN `h2`.
    h2: bool,
    /// Where the drop instant goes.
    closed: ClosedLog,
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
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
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
            },
            EndHandle(tx),
            closed,
        )
    }
}

impl ConnSource for ObservedSource {
    type Io = ObservedStream;

    async fn accept(&mut self) -> io::Result<Option<(ObservedStream, ConnMeta)>> {
        tokio::select! {
            accepted = self.listener.accept() => {
                let (sock, peer): (TcpStream, SocketAddr) = accepted?;
                Ok(Some((
                    ObservedStream { inner: sock, h2: self.h2, closed: self.closed.clone() },
                    ConnMeta::plain(peer),
                )))
            }
            _ = &mut self.end => Ok(None),
        }
    }
}
