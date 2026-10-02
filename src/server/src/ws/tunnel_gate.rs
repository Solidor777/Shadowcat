//! Backpressure for a WebSocket session tunnelled inside an HTTP/2 stream (RFC 8441 extended
//! CONNECT).
//!
//! WHY: hyper's tunnel task takes every chunk the session writes and hands it to h2 whether or not
//! the peer has granted send credit, and h2 buffers a write beyond the stream's window without
//! bound. A peer that returns no credit therefore never makes a write pending: the session keeps
//! producing, server memory grows with the room's traffic, and the stall bound of
//! [`BoundedSink`](crate::ws::bounded_sink::BoundedSink), which only runs while a send is pending,
//! never starts.
//!
//! [`TunnelGate`] makes the pending state real. It counts the bytes the session writes and
//! compares them with the DATA payload bytes the connection's writer has put on the wire for the
//! session's own stream ([`TunnelProbe`]); a session more than
//! [`H2_SEND_BUF_BYTES`] ahead of the wire is not
//! ready for its next frame, so the sink wrapped around the gate times the wait out with the same
//! window as every other send, and any progress of the wire releases it. The same bound, one send
//! buffer plus the frame being written, limits what a response body may be ahead of its peer.
//!
//! INVARIANT: the gate sits INSIDE the stall bound
//! (`BoundedSink<TunnelGate<_>>`, built by `WsState::bounded_sink`), never outside it, or a gated
//! wait would have no timer.

use std::pin::Pin;
use std::task::{Context, Poll};

use axum::extract::ws::Message;
use futures_util::Sink;

use crate::http::conn_serve::{TunnelProbe, H2_SEND_BUF_BYTES};

/// The bytes a server-to-client WebSocket frame of `msg` occupies on the wire: its payload and its
/// unmasked frame header (2 bytes, plus 2 or 8 of extended length). RFC 6455 section 5.2.
fn wire_len(msg: &Message) -> u64 {
    let payload = match msg {
        Message::Text(text) => text.as_str().len(),
        Message::Binary(bytes) | Message::Ping(bytes) | Message::Pong(bytes) => bytes.len(),
        Message::Close(frame) => frame.as_ref().map_or(0, |f| 2 + f.reason.as_str().len()),
    };
    let header = match payload {
        0..=125 => 2,
        126..=65535 => 4,
        _ => 10,
    };
    (payload + header) as u64
}

/// A sink that stops accepting frames while its session is further ahead of the wire than one h2
/// send buffer. See the module documentation.
///
/// # Examples
///
/// ```
/// use axum::extract::ws::Message;
/// use futures_util::SinkExt;
/// use shadowcat::ws::tunnel_gate::TunnelGate;
///
/// # #[tokio::main] async fn main() {
/// // Without a probe (HTTP/1, or a router served outside `serve_router`) the gate passes through.
/// let mut gate = TunnelGate::new(futures_util::sink::drain::<Message>(), None);
/// gate.send(Message::Text("hello".into())).await.unwrap();
/// # }
/// ```
pub struct TunnelGate<S> {
    /// The wrapped sink.
    inner: S,
    /// The session's stream on the connection's writer; `None` passes every frame through.
    probe: Option<TunnelProbe>,
    /// Wire bytes of the frames written so far.
    sent: u64,
}

impl<S> TunnelGate<S> {
    /// Wraps `inner`; `probe` is the session's own HTTP/2 stream, `None` on HTTP/1.
    ///
    /// # Examples
    ///
    /// ```
    /// use axum::extract::ws::Message;
    /// use shadowcat::ws::tunnel_gate::TunnelGate;
    ///
    /// let _gate = TunnelGate::new(futures_util::sink::drain::<Message>(), None);
    /// ```
    pub fn new(inner: S, probe: Option<TunnelProbe>) -> Self {
        Self {
            inner,
            probe,
            sent: 0,
        }
    }
}

impl<S> Sink<Message> for TunnelGate<S>
where
    S: Sink<Message> + Unpin,
{
    type Error = S::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        if let Some(probe) = &this.probe {
            if probe
                .poll_within(cx, this.sent, H2_SEND_BUF_BYTES as u64)
                .is_pending()
            {
                return Poll::Pending;
            }
        }
        Pin::new(&mut this.inner).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let this = self.get_mut();
        this.sent = this.sent.saturating_add(wire_len(&item));
        Pin::new(&mut this.inner).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}

#[cfg(test)]
mod tests;
