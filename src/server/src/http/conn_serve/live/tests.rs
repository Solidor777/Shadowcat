//! `LeasedBody`: the chunk bound ([`EGRESS_FLOOR_BYTES`]), exact size hints, pass-through of
//! non-data frames, and the stream marker the stream unit scans.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use http_body::Body as _;

use super::*;
use crate::http::conn_serve::stream_exec::{scoped, StreamExec};
use crate::http::conn_serve::EGRESS_FLOOR_BYTES;
use crate::http::peer_slots::PeerSlots;

/// A connection's live state over a fresh slot and fresh counters.
fn live() -> Arc<LiveState> {
    let slots = Arc::new(PeerSlots::new(4, 4));
    let ip: IpAddr = "192.0.2.1".parse().unwrap();
    LiveState::new(
        slots.admit(ip, false).unwrap(),
        Arc::new(StallCounters::default()),
    )
}

/// A body that yields a fixed list of results, then ends, with a hint derived from what remains.
struct Script {
    /// The results still to yield.
    frames: VecDeque<Result<Frame<Bytes>, axum::Error>>,
}

impl http_body::Body for Script {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Poll::Ready(self.frames.pop_front())
    }

    fn is_end_stream(&self) -> bool {
        self.frames.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        let total: u64 = self
            .frames
            .iter()
            .filter_map(|f| f.as_ref().ok()?.data_ref())
            .map(|d| d.len() as u64)
            .sum();
        SizeHint::with_exact(total)
    }
}

/// A leased body in `live` over `results`.
fn leased_results_in(
    live: &Arc<LiveState>,
    results: Vec<Result<Frame<Bytes>, axum::Error>>,
) -> LeasedBody {
    LeasedBody::new(
        Body::new(Script {
            frames: results.into(),
        }),
        live.lease(),
        live.clone(),
        None,
    )
}

/// A leased body in `live` over `frames`.
fn leased_in(live: &Arc<LiveState>, frames: Vec<Frame<Bytes>>) -> LeasedBody {
    leased_results_in(live, frames.into_iter().map(Ok).collect())
}

/// A leased body over `frames`, in its own connection state.
fn leased(frames: Vec<Frame<Bytes>>) -> LeasedBody {
    leased_in(&live(), frames)
}

/// Polls `body` once.
async fn next(body: &mut LeasedBody) -> Option<Result<Frame<Bytes>, axum::Error>> {
    std::future::poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)).await
}

#[tokio::test]
async fn a_one_mebibyte_chunk_is_yielded_as_sixty_four_frames_of_the_floor() {
    let mut body = leased(vec![Frame::data(Bytes::from(vec![7u8; 1 << 20]))]);
    let mut frames = 0usize;
    let mut delivered = 0u64;
    while let Some(frame) = next(&mut body).await {
        let data = frame.unwrap().into_data().unwrap();
        assert_eq!(data.len(), EGRESS_FLOOR_BYTES, "frame {frames}");
        delivered += data.len() as u64;
        frames += 1;
        let hint = body.size_hint();
        assert_eq!(
            hint.exact(),
            Some((1u64 << 20) - delivered),
            "the hint is exact after frame {frames}"
        );
    }
    assert_eq!(frames, 64);
    assert_eq!(delivered, 1 << 20);
}

#[tokio::test]
async fn the_end_of_stream_is_reported_only_after_the_last_pending_slice() {
    let mut body = leased(vec![Frame::data(Bytes::from(vec![
        1u8;
        3 * EGRESS_FLOOR_BYTES
    ]))]);
    assert!(!body.is_end_stream());
    for _ in 0..2 {
        next(&mut body).await.unwrap().unwrap();
        assert!(!body.is_end_stream(), "a slice is still pending");
    }
    next(&mut body).await.unwrap().unwrap();
    assert!(body.is_end_stream());
    assert!(next(&mut body).await.is_none());
}

#[tokio::test]
async fn a_frame_within_the_floor_passes_whole_and_the_remainder_follows() {
    let mut body = leased(vec![
        Frame::data(Bytes::from_static(b"abc")),
        Frame::data(Bytes::new()),
        Frame::data(Bytes::from(vec![2u8; EGRESS_FLOOR_BYTES + 5])),
    ]);
    let lens: Vec<usize> = {
        let mut v = Vec::new();
        while let Some(frame) = next(&mut body).await {
            v.push(frame.unwrap().into_data().unwrap().len());
        }
        v
    };
    assert_eq!(lens, vec![3, 0, EGRESS_FLOOR_BYTES, 5]);
}

#[tokio::test]
async fn trailers_pass_through_after_the_data_slices() {
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-end", axum::http::HeaderValue::from_static("1"));
    let mut body = leased(vec![
        Frame::data(Bytes::from(vec![3u8; EGRESS_FLOOR_BYTES + 1])),
        Frame::trailers(trailers),
    ]);
    assert_eq!(
        next(&mut body)
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .len(),
        EGRESS_FLOOR_BYTES
    );
    assert_eq!(
        next(&mut body)
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap()
            .len(),
        1
    );
    let last = next(&mut body).await.unwrap().unwrap();
    assert_eq!(last.trailers_ref().unwrap()["x-end"], "1");
    assert!(next(&mut body).await.is_none());
}

#[tokio::test]
async fn every_slice_shares_the_original_allocation() {
    let big = Bytes::from(vec![9u8; 2 * EGRESS_FLOOR_BYTES]);
    let base = big.as_ptr();
    let mut body = leased(vec![Frame::data(big)]);
    let first = next(&mut body).await.unwrap().unwrap().into_data().unwrap();
    let second = next(&mut body).await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(
        first.as_ptr(),
        base,
        "the first slice is a view, not a copy"
    );
    assert_eq!(
        second.as_ptr(),
        base.wrapping_add(EGRESS_FLOOR_BYTES),
        "the second slice is a view, not a copy"
    );
}

// ---------------------------------------------------------------------------------------------
// The stream marker
// ---------------------------------------------------------------------------------------------

/// One data frame of `n` bytes.
fn data(n: usize) -> Frame<Bytes> {
    Frame::data(Bytes::from(vec![5u8; n]))
}

/// Runs `f` inside a stream scope and returns its result.
async fn in_scope<T>(f: impl Future<Output = T>) -> T {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let (_cut, run) = scoped(async move {
        let _ = tx.send(f.await);
    });
    run.await;
    rx.await.unwrap()
}

/// The single live marker of `live`.
fn only_marker(live: &LiveState) -> Arc<StreamMarker> {
    let mut markers = live.live_markers();
    assert_eq!(markers.len(), 1, "expected exactly one live marker");
    markers.remove(0)
}

/// The live marker of `live` that currently holds a delivered frame.
fn marked(live: &LiveState) -> Arc<StreamMarker> {
    live.live_markers()
        .into_iter()
        .find(|m| m.delivered_at().is_some())
        .expect("a marker holding a delivered frame")
}

#[tokio::test]
async fn a_body_outside_a_stream_scope_registers_no_marker() {
    let live = live();
    let mut body = leased_in(&live, vec![data(10)]);
    next(&mut body).await.unwrap().unwrap();
    assert!(live.live_markers().is_empty());
    assert_eq!(live.marker_registry_len(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_data_frame_marks_the_stream_and_every_other_exit_clears_the_mark() {
    let live = live();
    in_scope(async {
        // Delivered, then cleared by the next poll's entry (which ends the body here).
        let mut body = leased_in(&live, vec![data(10)]);
        next(&mut body).await.unwrap().unwrap();
        let marker = only_marker(&live);
        assert!(marker.delivered_at().is_some(), "a delivered frame marks");
        assert!(next(&mut body).await.is_none());
        assert!(marker.delivered_at().is_none(), "Ready(None) clears");

        // Delivered, then the body is dropped.
        let mut body = leased_in(&live, vec![data(10)]);
        next(&mut body).await.unwrap().unwrap();
        let marker = marked(&live);
        drop(body);
        assert!(marker.delivered_at().is_none(), "Drop clears");

        // Delivered, then an error from the inner body.
        let mut body = leased_results_in(
            &live,
            vec![
                Ok(data(10)),
                Err(axum::Error::new(std::io::Error::other("inner failed"))),
            ],
        );
        next(&mut body).await.unwrap().unwrap();
        let marker = marked(&live);
        assert!(next(&mut body).await.unwrap().is_err());
        assert!(marker.delivered_at().is_none(), "Err clears");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn an_empty_data_frame_and_trailers_leave_the_marker_clear() {
    let live = live();
    in_scope(async {
        let mut trailers = axum::http::HeaderMap::new();
        trailers.insert("x-end", axum::http::HeaderValue::from_static("1"));
        let mut body = leased_in(
            &live,
            vec![Frame::data(Bytes::new()), Frame::trailers(trailers)],
        );
        next(&mut body).await.unwrap().unwrap();
        let marker = only_marker(&live);
        assert!(
            marker.delivered_at().is_none(),
            "an empty frame waits for no credit"
        );
        next(&mut body).await.unwrap().unwrap();
        assert!(
            marker.delivered_at().is_none(),
            "trailers wait for no credit"
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn every_slice_of_a_large_frame_marks_the_stream() {
    let live = live();
    in_scope(async {
        let mut body = leased_in(&live, vec![data(2 * EGRESS_FLOOR_BYTES)]);
        next(&mut body).await.unwrap().unwrap();
        let marker = only_marker(&live);
        let first = marker.delivered_at().unwrap();
        tokio::time::advance(Duration::from_millis(30)).await;
        next(&mut body).await.unwrap().unwrap();
        let second = marker.delivered_at().expect("the second slice marks");
        assert!(second > first, "the second slice restarts the age");
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn a_chunk_waiting_for_the_window_is_cut_once_and_counted_as_a_stream_cut() {
    let live = live();
    let window = Duration::from_secs(10);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let l = live.clone();
    StreamExec.execute_test(async move {
        let mut body = leased_in(&l, vec![data(10), data(10)]);
        next(&mut body).await.unwrap().unwrap();
        let _ = ready_tx.send(());
        std::future::pending::<()>().await;
    });
    ready_rx.await.unwrap();
    let marker = only_marker(&live);
    let delivered = marker.delivered_at().unwrap();

    tokio::time::advance(window - Duration::from_millis(1)).await;
    assert_eq!(live.cut_stalled_streams(Instant::now(), window), 0);
    assert!(!marker.cut.has_fired(), "cut before the window elapsed");

    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(Instant::now() - delivered, window);
    assert_eq!(live.cut_stalled_streams(Instant::now(), window), 1);
    assert!(marker.cut.has_fired());
    assert_eq!(live.stall_counts().stream, 1);

    // The mark is consumed, so a scan before the cut's task has run (nothing has yielded to the
    // runtime yet, so the body is still alive) does not count the same stall again.
    assert_eq!(live.cut_stalled_streams(Instant::now(), window), 0);
    assert_eq!(live.stall_counts().stream, 1);
    tokio::time::advance(window).await;
    assert_eq!(live.cut_stalled_streams(Instant::now(), window), 0);
    assert_eq!(live.stall_counts().stream, 1);
}

#[tokio::test(start_paused = true)]
async fn a_body_the_peer_keeps_asking_past_is_never_cut() {
    let live = live();
    let window = Duration::from_secs(10);
    in_scope(async {
        let mut body = leased_in(&live, (0..20).map(|_| data(1)).collect());
        for _ in 0..20 {
            next(&mut body).await.unwrap().unwrap();
            tokio::time::advance(window / 2).await;
            assert_eq!(live.cut_stalled_streams(Instant::now(), window), 0);
        }
        assert_eq!(live.stall_counts().stream, 0);
    })
    .await;
}

/// Sets its flag when dropped.
struct DropGuard(Arc<AtomicBool>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn a_stream_cut_drops_its_stream_task_and_spares_a_tunnel_shaped_sibling() {
    let live = live();
    let window = Duration::from_secs(10);
    let stalled_dropped = Arc::new(AtomicBool::new(false));
    let sibling_dropped = Arc::new(AtomicBool::new(false));

    // A response stream: delivers one chunk and then waits on the peer.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (flag, l) = (DropGuard(stalled_dropped.clone()), live.clone());
    StreamExec.execute_test(async move {
        let _flag = flag;
        let mut body = leased_in(&l, vec![data(10), data(10)]);
        next(&mut body).await.unwrap().unwrap();
        let _ = ready_tx.send(());
        std::future::pending::<()>().await;
    });
    // A tunnel-shaped stream: no response body, so no marker.
    let flag = DropGuard(sibling_dropped.clone());
    StreamExec.execute_test(async move {
        let _flag = flag;
        std::future::pending::<()>().await;
    });
    ready_rx.await.unwrap();

    tokio::time::advance(window).await;
    assert_eq!(live.cut_stalled_streams(Instant::now(), window), 1);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        stalled_dropped.load(Ordering::SeqCst),
        "the cut did not drop the stalled stream's task"
    );
    assert!(
        !sibling_dropped.load(Ordering::SeqCst),
        "a stream with no response body was cut"
    );
}

#[tokio::test]
async fn the_marker_registry_stays_bounded_over_many_short_streams_and_keeps_long_lived_ones() {
    use crate::http::conn_serve::pruned::PRUNE_FLOOR;
    let live = live();
    let bound = (2 * 3).max(PRUNE_FLOOR) + 1;
    in_scope(async {
        // Three streams stay open for the whole run.
        let mut held: Vec<LeasedBody> = Vec::new();
        for _ in 0..3 {
            let mut body = leased_in(&live, vec![data(1), data(1)]);
            next(&mut body).await.unwrap().unwrap();
            held.push(body);
        }
        let long_lived = live.live_markers();
        assert_eq!(long_lived.len(), 3);
        let mut peak = 0;
        for _ in 0..100_000 {
            let mut short = leased_in(&live, vec![data(1)]);
            next(&mut short).await.unwrap().unwrap();
            peak = peak.max(live.marker_registry_len());
        }
        assert!(
            peak <= bound,
            "the registry reached {peak} entries; bound {bound}"
        );
        let survivors = live.live_markers();
        assert_eq!(survivors.len(), 3, "a prune dropped a long-lived marker");
        for m in &long_lived {
            assert!(survivors.iter().any(|s| Arc::ptr_eq(s, m)));
        }
        drop(held);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// The stream table: registration at the END_STREAM-bearing frame and hand-over at the drop
// ---------------------------------------------------------------------------------------------

/// The client connection preface.
const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// A request HEADERS frame (END_STREAM | END_HEADERS) on `sid`.
fn request_frame(sid: u32) -> Vec<u8> {
    let payload = [0x82u8, 0x87, 0x84];
    let mut out = vec![0, 0, payload.len() as u8, 0x1, 0x5];
    out.extend_from_slice(&sid.to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

/// A connection state whose stream table holds an `Open` entry for each of `sids`, as the read
/// parser creates them.
fn live_with_streams(sids: &[u32]) -> Arc<LiveState> {
    let live = live();
    let mut reads =
        crate::http::conn_serve::egress::WriteCounter::new(live.egress(), true).read_observer();
    let kill = crate::http::conn_serve::kill::ConnKill::new();
    reads.after_read(PREFACE, &kill);
    for sid in sids {
        reads.after_read(&request_frame(*sid), &kill);
    }
    live
}

/// A leased body over `frames` for stream `sid` in `live`.
fn leased_for(live: &Arc<LiveState>, sid: Option<u32>, frames: Vec<Frame<Bytes>>) -> LeasedBody {
    LeasedBody::new(
        Body::new(Script {
            frames: frames.into_iter().map(Ok).collect(),
        }),
        live.lease(),
        live.clone(),
        sid,
    )
}

/// The phase of stream `sid` in `live`'s table.
fn phase(live: &Arc<LiveState>, sid: u32) -> Option<&'static str> {
    live.egress()
        .stream_phases()
        .into_iter()
        .find(|(id, _)| *id == sid)
        .map(|(_, phase)| phase)
}

/// A body whose inner reports no end of stream until it has returned `None`, as a streamed body
/// does.
struct EndsWithNone(Script);

impl http_body::Body for EndsWithNone {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.0).poll_frame(cx)
    }
}

#[tokio::test]
async fn the_body_registers_final_exactly_when_it_returns_the_frame_that_carries_end_of_stream() {
    let live = live_with_streams(&[1]);
    let mut body = leased_for(
        &live,
        Some(1),
        vec![
            Frame::data(Bytes::from_static(b"one")),
            Frame::data(Bytes::from_static(b"two")),
            Frame::data(Bytes::from_static(b"last")),
        ],
    );
    assert_eq!(phase(&live, 1), Some("open"));
    next(&mut body).await.unwrap().unwrap();
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(
        phase(&live, 1),
        Some("open"),
        "registered before the last frame"
    );
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(phase(&live, 1), Some("final"));
}

#[tokio::test]
async fn a_chunk_that_is_not_the_last_slice_of_its_frame_does_not_register() {
    let live = live_with_streams(&[1]);
    let mut body = leased_for(
        &live,
        Some(1),
        vec![Frame::data(Bytes::from(vec![0u8; EGRESS_FLOOR_BYTES + 1]))],
    );
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(
        phase(&live, 1),
        Some("open"),
        "a slice with more pending is not final"
    );
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(phase(&live, 1), Some("final"));
}

#[tokio::test]
async fn a_body_that_ends_with_none_registers_final_at_the_none_and_not_at_its_last_data() {
    let live = live_with_streams(&[1]);
    let mut body = LeasedBody::new(
        Body::new(EndsWithNone(Script {
            frames: [Ok(Frame::data(Bytes::from_static(b"data")))].into(),
        })),
        live.lease(),
        live.clone(),
        Some(1),
    );
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(phase(&live, 1), Some("open"));
    assert!(next(&mut body).await.is_none());
    assert_eq!(phase(&live, 1), Some("final"));
}

#[tokio::test]
async fn a_body_that_ends_with_trailers_registers_final_at_the_trailers() {
    let live = live_with_streams(&[1]);
    let mut trailers = axum::http::HeaderMap::new();
    trailers.insert("x-end", axum::http::HeaderValue::from_static("1"));
    let mut body = leased_for(
        &live,
        Some(1),
        vec![
            Frame::data(Bytes::from_static(b"data")),
            Frame::trailers(trailers),
        ],
    );
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(phase(&live, 1), Some("open"));
    next(&mut body).await.unwrap().unwrap();
    assert_eq!(phase(&live, 1), Some("final"));
}

#[tokio::test]
async fn dropping_a_final_body_queues_its_entry_and_a_body_dropped_earlier_leaves_it_open() {
    let live = live_with_streams(&[1, 3]);
    let mut finished = leased_for(
        &live,
        Some(1),
        vec![Frame::data(Bytes::from_static(b"abc"))],
    );
    next(&mut finished).await.unwrap().unwrap();
    assert_eq!(finished.handed, 3);
    let early = leased_for(
        &live,
        Some(3),
        vec![Frame::data(Bytes::from_static(b"abc"))],
    );
    drop(finished);
    drop(early);
    assert_eq!(phase(&live, 1), Some("queued"));
    assert_eq!(phase(&live, 3), Some("open"));
}

#[tokio::test]
async fn a_body_with_no_stream_id_never_touches_the_table() {
    let live = live_with_streams(&[1]);
    let mut body = leased_for(&live, None, vec![Frame::data(Bytes::from_static(b"abc"))]);
    next(&mut body).await.unwrap().unwrap();
    drop(body);
    assert_eq!(phase(&live, 1), Some("open"));
}

#[tokio::test]
async fn a_body_for_a_stream_the_wire_already_ended_creates_no_entry() {
    let live = live_with_streams(&[]);
    let mut body = leased_for(
        &live,
        Some(1),
        vec![Frame::data(Bytes::from_static(b"abc"))],
    );
    next(&mut body).await.unwrap().unwrap();
    drop(body);
    assert_eq!(phase(&live, 1), None);
}
