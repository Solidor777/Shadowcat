//! These tests drive `TunnelGate` against a stream table that the test feeds as the connection's
//! writer would.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Wake, Waker};

use futures_util::SinkExt;

use super::*;

/// A sink that accepts every frame and records how many.
struct Taken(Arc<AtomicUsize>);

impl Sink<Message> for Taken {
    type Error = ();

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), ()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
        Poll::Ready(Ok(()))
    }
}

/// A waker that counts its wakes.
struct Count(AtomicUsize);

impl Wake for Count {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A frame whose wire size is `payload + 4` (a payload from 126 to 65,535 bytes).
fn frame(payload: usize) -> Message {
    Message::Binary(vec![0u8; payload].into())
}

/// Polls readiness once with a counting waker; returns the result and the count.
fn poll_ready(gate: &mut TunnelGate<Taken>) -> (Poll<Result<(), ()>>, Arc<Count>) {
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    (Pin::new(gate).poll_ready(&mut cx), count)
}

#[test]
fn wire_length_counts_the_payload_and_the_unmasked_header() {
    assert_eq!(wire_len(&frame(0)), 2);
    assert_eq!(wire_len(&frame(125)), 127);
    assert_eq!(wire_len(&frame(126)), 130);
    assert_eq!(wire_len(&frame(65_535)), 65_539);
    assert_eq!(wire_len(&frame(65_536)), 65_546);
    assert_eq!(wire_len(&Message::Text("abc".into())), 5);
    assert_eq!(wire_len(&Message::Close(None)), 2);
}

#[tokio::test]
async fn without_a_probe_the_gate_passes_every_frame() {
    let taken = Arc::new(AtomicUsize::new(0));
    let mut gate = TunnelGate::new(Taken(taken.clone()), None);
    for _ in 0..100 {
        gate.send(frame(60_000)).await.unwrap();
    }
    assert_eq!(taken.load(Ordering::SeqCst), 100);
}

#[test]
fn a_session_more_than_a_send_buffer_ahead_of_the_wire_is_not_ready_until_the_wire_moves() {
    let probe = TunnelProbe::detached(1);
    let writer = probe.clone();
    let taken = Arc::new(AtomicUsize::new(0));
    let mut gate = TunnelGate::new(Taken(taken.clone()), Some(probe));
    // Frames of 60,000 payload bytes: the first leaves the session 60,004 ahead (within the 65,536
    // cap), the second 120,008 ahead.
    assert!(matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))));
    Pin::new(&mut gate).start_send(frame(60_000)).unwrap();
    assert!(matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))));
    Pin::new(&mut gate).start_send(frame(60_000)).unwrap();
    let (pending, count) = poll_ready(&mut gate);
    assert!(
        pending.is_pending(),
        "120,008 bytes ahead of a wire that has taken none"
    );
    assert_eq!(count.0.load(Ordering::SeqCst), 0);
    // The wire takes enough to bring the session back within a send buffer: the parked task wakes
    // and the gate is ready again.
    writer.written_on_wire(60_000);
    assert_eq!(
        count.0.load(Ordering::SeqCst),
        1,
        "the wire's progress woke the parked task"
    );
    assert!(matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))));
    assert_eq!(taken.load(Ordering::SeqCst), 2);
}

#[test]
fn a_wire_that_takes_too_little_leaves_the_session_pending_and_wakes_it_each_time() {
    let probe = TunnelProbe::detached(1);
    let writer = probe.clone();
    let mut gate = TunnelGate::new(Taken(Arc::new(AtomicUsize::new(0))), Some(probe));
    Pin::new(&mut gate).start_send(frame(65_535)).unwrap();
    Pin::new(&mut gate).start_send(frame(65_535)).unwrap();
    let (first, count) = poll_ready(&mut gate);
    assert!(first.is_pending());
    writer.written_on_wire(10);
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    let (second, count) = poll_ready(&mut gate);
    assert!(
        second.is_pending(),
        "10 bytes of progress does not clear a 131,078-byte backlog"
    );
    writer.written_on_wire(10);
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
}

#[test]
fn the_end_of_the_stream_wakes_a_parked_session() {
    let probe = TunnelProbe::detached(1);
    let writer = probe.clone();
    let mut gate = TunnelGate::new(Taken(Arc::new(AtomicUsize::new(0))), Some(probe));
    Pin::new(&mut gate).start_send(frame(65_535)).unwrap();
    Pin::new(&mut gate).start_send(frame(65_535)).unwrap();
    let (pending, count) = poll_ready(&mut gate);
    assert!(pending.is_pending());
    writer.ended_on_wire();
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
    assert!(
        matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))),
        "an ended stream is not parked: the next write reports it"
    );
}
