//! HTTP/2 with prior knowledge over plain TCP: a serve-loop source and a client.
//!
//! HTTP/2 with prior knowledge over plain TCP, for driving `http::conn_serve` with an HTTP/2
//! connection (extended-CONNECT WebSockets included) without a TLS handshake: a
//! [`ConnSource`] whose connections all claim ALPN `h2`, and a client that speaks HTTP/2 from the
//! first byte.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use shadowcat::http::conn_serve::{set_accepted_nodelay, ConnSource, ServeIo};
use shadowcat::tls::ConnMeta;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::tls::H2Client;

/// A TCP stream that reports ALPN `h2`, standing in for a TLS stream that negotiated it.
pub struct H2cStream(TcpStream);

impl AsyncRead for H2cStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for H2cStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl ServeIo for H2cStream {
    fn is_h2(&self) -> bool {
        true
    }
}

/// A [`ConnSource`] over a real listener whose connections all speak HTTP/2, and which ends when
/// its [`EndHandle`] fires.
pub struct H2cSource {
    /// The bound listener.
    listener: TcpListener,
    /// Fires when the source should report that it has ended.
    end: oneshot::Receiver<()>,
    /// Whether accepted sockets get `TCP_NODELAY` through `set_accepted_nodelay`, the function
    /// `impl ConnSource for TcpListener` and `SniffListener`'s accept task call.
    nodelay: bool,
}

/// Ends its [`H2cSource`] when [`EndHandle::end`] is called.
pub struct EndHandle(pub(crate) oneshot::Sender<()>);

impl EndHandle {
    /// Makes the paired source return `None` from its next `accept`, as a dead acceptor would.
    pub fn end(self) {
        let _ = self.0.send(());
    }
}

impl H2cSource {
    /// A source over `listener` and the handle that ends it.
    pub fn new(listener: TcpListener) -> (Self, EndHandle) {
        let (tx, end) = oneshot::channel();
        (
            Self {
                listener,
                end,
                nodelay: true,
            },
            EndHandle(tx),
        )
    }

    /// Sets whether accepted sockets get `TCP_NODELAY` (on by default, as in production); the
    /// throughput ladder turns it off for its control.
    pub fn with_nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = nodelay;
        self
    }
}

impl ConnSource for H2cSource {
    type Io = H2cStream;

    async fn accept(&mut self) -> io::Result<Option<(H2cStream, ConnMeta)>> {
        tokio::select! {
            accepted = self.listener.accept() => {
                let (sock, peer) = accepted?;
                if self.nodelay {
                    set_accepted_nodelay(&sock, peer);
                }
                Ok(Some((H2cStream(sock), ConnMeta::plain(peer))))
            }
            _ = &mut self.end => Ok(None),
        }
    }
}

/// Connects to `addr` speaking HTTP/2 from the first byte, with `builder`'s settings, and spawns
/// the connection driver. The returned client's `driver` finishes when the connection ends.
pub async fn h2c_connect(addr: SocketAddr, builder: h2::client::Builder) -> io::Result<H2Client> {
    let stream = TcpStream::connect(addr).await?;
    let (sender, conn) = builder.handshake(stream).await.map_err(io::Error::other)?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(H2Client { sender, driver })
}
