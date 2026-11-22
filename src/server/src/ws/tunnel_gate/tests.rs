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

/// What a gate reports once it has been written to until it is no longer ready.
struct Parked {
    /// The wake count of the pending readiness poll.
    count: Arc<Count>,
    /// How many frames were written.
    frames: usize,
    /// The wire bytes of those frames.
    wire: u64,
}

/// Writes frames of `payload` bytes while the gate is ready; returns at the first pending
/// readiness poll. The session ends more than `H2_SEND_BUF_BYTES` ahead of a wire that has taken
/// nothing, by less than one frame.
fn write_until_parked(gate: &mut TunnelGate<Taken>, payload: usize) -> Parked {
    let mut frames = 0;
    let mut wire = 0;
    loop {
        let (poll, count) = poll_ready(gate);
        if poll.is_pending() {
            assert!(
                wire > H2_SEND_BUF_BYTES as u64,
                "pending at {wire} bytes, within the send buffer"
            );
            return Parked {
                count,
                frames,
                wire,
            };
        }
        assert!(matches!(poll, Poll::Ready(Ok(()))));
        Pin::new(&mut *gate).start_send(frame(payload)).unwrap();
        frames += 1;
        wire += wire_len(&frame(payload));
    }
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
    // Frames of 60,000 payload bytes until the session is more than a send buffer ahead of a wire
    // that has taken none.
    let parked = write_until_parked(&mut gate, 60_000);
    assert_eq!(parked.count.0.load(Ordering::SeqCst), 0);
    // The wire takes one frame, which brings the session (over the buffer by less than one frame)
    // back within it: the parked task wakes and the gate is ready again.
    writer.written_on_wire(wire_len(&frame(60_000)));
    assert_eq!(
        parked.count.0.load(Ordering::SeqCst),
        1,
        "the wire's progress woke the parked task"
    );
    assert!(matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))));
    assert_eq!(taken.load(Ordering::SeqCst), parked.frames);
}

#[test]
fn a_wire_that_takes_too_little_leaves_the_session_pending_and_wakes_it_each_time() {
    let probe = TunnelProbe::detached(1);
    let writer = probe.clone();
    let mut gate = TunnelGate::new(Taken(Arc::new(AtomicUsize::new(0))), Some(probe));
    let parked = write_until_parked(&mut gate, 65_535);
    let backlog = parked.wire - H2_SEND_BUF_BYTES as u64;
    assert!(
        backlog > 20,
        "the premise: the backlog dwarfs the 10 bytes taken"
    );
    writer.written_on_wire(10);
    assert_eq!(parked.count.0.load(Ordering::SeqCst), 1);
    let (second, count) = poll_ready(&mut gate);
    assert!(
        second.is_pending(),
        "10 bytes of progress does not clear a {backlog}-byte backlog"
    );
    writer.written_on_wire(10);
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
}

#[test]
fn the_end_of_the_stream_wakes_a_parked_session() {
    let probe = TunnelProbe::detached(1);
    let writer = probe.clone();
    let mut gate = TunnelGate::new(Taken(Arc::new(AtomicUsize::new(0))), Some(probe));
    let parked = write_until_parked(&mut gate, 65_535);
    writer.ended_on_wire();
    assert_eq!(parked.count.0.load(Ordering::SeqCst), 1);
    assert!(
        matches!(poll_ready(&mut gate).0, Poll::Ready(Ok(()))),
        "an ended stream is not parked: the next write reports it"
    );
}
