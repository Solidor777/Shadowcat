//! The stream table against a real HTTP/2 server codec behind a connection whose reads and writes
//! the test controls: a minimum-frame flood (the parse lag), the fail-closed cap, a drain flood
//! after GOAWAY, and a request reset in the same write that carried it.
//!
//! Each test serves one connection through `spawn_connection` over a duplex pipe whose server end
//! records the largest single read and can park its writer, so the premises of each derived bound
//! are measured in the same test that asserts the bound.

use std::sync::atomic::AtomicBool;
use std::task::{Context, Poll};

use axum::routing::get;
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio::time::timeout;

use super::*;
use crate::http::peer_slots::PeerSlots;

/// The client connection preface.
const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// The size of the codec's read buffer the lag derivation assumes: one read can carry at most this
/// many bytes of frames.
const READ_BUFFER_BYTES: usize = 8 * 1024;

/// The server end of a duplex pipe that reports ALPN `h2`, records the largest single read and can
/// park its writer.
struct Probed {
    /// The pipe's server end.
    inner: DuplexStream,
    /// The most bytes any one read returned.
    max_read: Arc<AtomicUsize>,
    /// While set, every write and flush stays pending (a peer that has stopped reading).
    block_write: Arc<AtomicBool>,
}

impl AsyncRead for Probed {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = result {
            self.max_read
                .fetch_max(buf.filled().len() - before, Ordering::SeqCst);
        }
        result
    }
}

impl AsyncWrite for Probed {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.block_write.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.block_write.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl ServeIo for Probed {
    fn is_h2(&self) -> bool {
        true
    }
}

/// One served connection and the handles a test drives it through.
struct Served {
    /// The client's end of the pipe.
    peer: DuplexStream,
    /// The connection's liveness state, whose egress record holds the stream table.
    live: Arc<LiveState>,
    /// The serve loop's cut counters.
    counters: Arc<StallCounters>,
    /// The largest single read the server's codec performed.
    max_read: Arc<AtomicUsize>,
    /// Parks the server's writer while set.
    block_write: Arc<AtomicBool>,
    /// The connection task.
    tasks: JoinSet<()>,
    /// The drain signal's sender; the drain begins when `hold`, its only receiver, drops.
    _signal_tx: watch::Sender<()>,
    /// The drain deadline's kill switch.
    kill: KillSignal,
    /// Connections `drive` force-dropped.
    dropped: Arc<AtomicUsize>,
    /// Keeps the drain signal open until the test drops it.
    hold: Option<watch::Receiver<()>>,
}

/// The routes the tests request: `/` answers at once and `/hang` never does.
fn app() -> Router {
    Router::new()
        .route("/", get(|| async { "ok" }))
        .route("/hang", get(std::future::pending::<&'static str>))
}

/// Serves one HTTP/2 connection whose stream table holds at most `cap` entries.
fn serve(cap: usize) -> Served {
    let (peer, server_end) = tokio::io::duplex(1 << 20);
    let max_read = Arc::new(AtomicUsize::new(0));
    let block_write = Arc::new(AtomicBool::new(false));
    let probed = Probed {
        inner: server_end,
        max_read: max_read.clone(),
        block_write: block_write.clone(),
    };
    let counters = Arc::new(StallCounters::default());
    let slots = Arc::new(PeerSlots::new(4, 4));
    let slot = slots.admit("192.0.2.1".parse().unwrap(), false).unwrap();
    let live = LiveState::with_table_cap(slot, counters.clone(), cap);
    let (signal_tx, hold) = watch::channel(());
    let kill = KillSignal::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    spawn_connection(
        &mut tasks,
        probed,
        ConnMeta::plain("192.0.2.1:4000".parse().unwrap()),
        app(),
        live.clone(),
        Ending {
            signal_tx: &signal_tx,
            kill: &kill,
            dropped: &dropped,
        },
        ConnLimits {
            drain_timeout: Duration::from_secs(30),
            ..ConnLimits::default()
        },
    );
    Served {
        peer,
        live,
        counters,
        max_read,
        block_write,
        tasks,
        _signal_tx: signal_tx,
        kill,
        dropped,
        hold: Some(hold),
    }
}

/// One HTTP/2 frame.
fn frame(kind: u8, flags: u8, sid: u32, payload: &[u8]) -> Vec<u8> {
    let n = payload.len() as u32;
    let mut out = vec![(n >> 16) as u8, (n >> 8) as u8, n as u8, kind, flags];
    out.extend_from_slice(&sid.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A request HEADERS frame (END_STREAM | END_HEADERS) on `sid` for `GET /` (a 12-byte frame) or,
/// with `minimum`, with an empty header block (the 9-byte smallest frame).
fn request(sid: u32, minimum: bool) -> Vec<u8> {
    let block: &[u8] = if minimum { &[] } else { &[0x82, 0x87, 0x84] };
    frame(0x1, 0x5, sid, block)
}

impl Served {
    /// Sends the preface and an empty SETTINGS, and reads until the server's own SETTINGS arrive
    /// so the connection is established. Returns the settings the server advertised as
    /// `(identifier, value)` pairs.
    async fn open(&mut self) -> Vec<(u16, u32)> {
        let mut hello = PREFACE.to_vec();
        hello.extend(frame(0x4, 0, 0, &[]));
        self.peer.write_all(&hello).await.unwrap();
        let mut head = [0u8; 9];
        timeout(Duration::from_secs(5), self.peer.read_exact(&mut head))
            .await
            .expect("the server sent nothing")
            .unwrap();
        assert_eq!(head[3], 0x4, "the server's first frame is SETTINGS");
        let mut rest = vec![0u8; u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize];
        self.peer.read_exact(&mut rest).await.unwrap();
        let (settings, _) = rest.as_chunks::<6>();
        settings
            .iter()
            .map(|setting| {
                (
                    u16::from_be_bytes([setting[0], setting[1]]),
                    u32::from_be_bytes([setting[2], setting[3], setting[4], setting[5]]),
                )
            })
            .collect()
    }

    /// Reads and discards everything the server sends, on a task, and returns the writing half.
    fn drain_reads(&mut self) -> tokio::io::WriteHalf<DuplexStream> {
        let (mut read, write) =
            tokio::io::split(std::mem::replace(&mut self.peer, tokio::io::duplex(1).0));
        tokio::spawn(async move {
            let mut sink = [0u8; 16 * 1024];
            while matches!(read.read(&mut sink).await, Ok(n) if n > 0) {}
        });
        write
    }

    /// Waits until the stream table holds exactly `ids`.
    async fn wait_for_ids(&self, ids: &[u32]) {
        let live = self.live.clone();
        let want = ids.to_vec();
        timeout(Duration::from_secs(10), async move {
            while live.egress().stream_ids() != want {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the table is {:?}, not {ids:?}",
                self.live.egress().stream_ids()
            )
        });
    }

    /// Ends the connection and its task.
    async fn finish(mut self) {
        self.hold = None;
        self.kill.fire();
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
    }
}

/// Floods `n` request frames of the given kind in one write on the connection and returns the
/// served connection after the flood has been read and processed.
async fn flood(blocked: bool, minimum: bool, n: u32) -> Served {
    let mut served = serve(STREAM_TABLE_CAP_FOR_TESTS);
    served.open().await;
    let mut write = served.drain_reads();
    if blocked {
        served.block_write.store(true, Ordering::SeqCst);
    }
    let mut burst = Vec::new();
    for i in 0..n {
        burst.extend(request(1 + 2 * i, minimum));
    }
    let writer = tokio::spawn(async move {
        let _ = write.write_all(&burst).await;
        write
    });
    // Everything has been read once the codec has stopped reading for a while (a blocked writer
    // makes it stop early: it cannot queue more refusals).
    let mut last = usize::MAX;
    let mut steady = 0;
    while steady < 8 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = served.counters.table_peak();
        steady = if now == last { steady + 1 } else { 0 };
        last = now;
    }
    writer.abort();
    served
}

/// The production cap, which the lag tests run under.
const STREAM_TABLE_CAP_FOR_TESTS: usize = egress::STREAM_TABLE_CAP;

#[tokio::test]
async fn the_parse_lag_over_a_minimum_frame_flood_stays_inside_its_derived_slack() {
    for minimum in [false, true] {
        for blocked in [true, false] {
            let served = flood(blocked, minimum, 20_000).await;
            let frame_bytes = if minimum { 9 } else { 12 };
            let max_read = served.max_read.load(Ordering::SeqCst);
            let peak = served.counters.table_peak();
            // Premises of the derivation, named so a change in the codec's read or frame sizes
            // fails here and not on a magnitude: one read carries at most the read buffer, and the
            // flooded frame is no smaller than the minimum the derivation assumes.
            assert!(
                max_read <= READ_BUFFER_BYTES,
                "a single read of {max_read} B exceeds the {READ_BUFFER_BYTES} B read buffer the slack assumes"
            );
            assert!(frame_bytes >= 9, "the flooded frame is below the minimum");
            // The lag is the entries the table holds above the streams h2 admits: bounded by the
            // slack, which is five reads' worth of minimum frames.
            let lag = peak.saturating_sub(H2_MAX_CONCURRENT_STREAMS as usize);
            assert!(
                lag <= egress::PARSE_LAG_SLACK,
                "minimum {minimum}, writer blocked {blocked}: lag {lag} above the slack {}",
                egress::PARSE_LAG_SLACK
            );
            assert_eq!(
                served.counters.snapshot(),
                StallCuts::default(),
                "minimum {minimum}, blocked {blocked}: a flood within the lag cut the connection"
            );
            println!(
                "RECORD parse_lag frame_bytes={frame_bytes} writer_blocked={blocked} max_read_bytes={max_read} table_peak={peak} lag={lag} slack={}",
                egress::PARSE_LAG_SLACK
            );
            served.finish().await;
        }
    }
}

#[tokio::test]
async fn a_table_past_its_cap_against_a_parked_writer_cuts_the_connection_exactly_once() {
    let mut served = serve(10 + 20);
    served.open().await;
    let mut write = served.drain_reads();
    served.block_write.store(true, Ordering::SeqCst);
    let mut burst = Vec::new();
    for i in 0..200u32 {
        burst.extend(request(1 + 2 * i, false));
    }
    write.write_all(&burst).await.unwrap();
    timeout(Duration::from_secs(10), async {
        while served.counters.snapshot().tracker_overflow == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the overflow never cut the connection");
    served.block_write.store(false, Ordering::SeqCst);
    timeout(Duration::from_secs(10), async {
        while served.tasks.join_next().await.is_some() {}
    })
    .await
    .expect("the cut connection did not end");
    assert_eq!(
        served.counters.snapshot(),
        StallCuts {
            tracker_overflow: 1,
            ..StallCuts::default()
        }
    );
    assert!(
        served.counters.table_peak() <= 30,
        "the table grew to {}",
        served.counters.table_peak()
    );
    assert_eq!(
        served.dropped.load(Ordering::SeqCst),
        0,
        "the IO failure ends it"
    );
}

/// Reads server frames until `want` returns true for one, answering every PING with its
/// acknowledgement, and returns the frame's type, flags and payload.
async fn read_until(served: &mut Served, mut want: impl FnMut(u8, &[u8]) -> bool) -> (u8, Vec<u8>) {
    timeout(Duration::from_secs(10), async {
        loop {
            let mut head = [0u8; 9];
            served.peer.read_exact(&mut head).await.unwrap();
            let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0u8; len];
            served.peer.read_exact(&mut payload).await.unwrap();
            if head[3] == 0x6 && head[4] & 1 == 0 {
                served
                    .peer
                    .write_all(&frame(0x6, 0x1, 0, &payload))
                    .await
                    .unwrap();
            }
            if want(head[3], &payload) {
                return (head[3], payload);
            }
        }
    })
    .await
    .expect("the frame never arrived")
}

#[tokio::test]
async fn a_request_flood_after_the_goaway_never_grows_the_table() {
    let mut served = serve(egress::STREAM_TABLE_CAP);
    served.open().await;
    // Stream 1 hangs, so the connection outlives the graceful GOAWAY.
    let mut hang = vec![0x82, 0x87, 0x04, 0x05];
    hang.extend_from_slice(b"/hang");
    served
        .peer
        .write_all(&frame(0x1, 0x5, 1, &hang))
        .await
        .unwrap();
    served.wait_for_ids(&[1]).await;
    // Begin the drain; the server writes GOAWAY(2^31 - 1), a PING, and after the PONG its final
    // GOAWAY with last stream id 1. Answer the PING while reading.
    served.hold = None;
    let (_, first) = read_until(&mut served, |kind, _| kind == 0x7).await;
    assert_eq!(
        u32::from_be_bytes([first[0], first[1], first[2], first[3]]),
        0x7fff_ffff
    );
    let (_, last) = read_until(&mut served, |kind, _| kind == 0x7).await;
    assert_eq!(u32::from_be_bytes([last[0], last[1], last[2], last[3]]), 1);
    // The final GOAWAY is written: ten caps' worth of requests above it must add nothing.
    let flood_count = 10 * egress::STREAM_TABLE_CAP as u32;
    let mut burst = Vec::new();
    for i in 0..flood_count {
        burst.extend(request(3 + 2 * i, false));
    }
    let mut write = served.drain_reads();
    write.write_all(&burst).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(served.live.egress().stream_ids(), [1]);
    assert_eq!(served.counters.snapshot(), StallCuts::default());
    assert!(
        served.counters.table_peak() <= 1,
        "the flood grew the table to {}",
        served.counters.table_peak()
    );
    served.finish().await;
}

#[tokio::test]
async fn a_request_reset_in_the_same_write_that_carried_it_leaves_no_entry() {
    // HEADERS(1) and RST_STREAM(1) arrive in one read: the parse-time entry is created and then
    // removed by the reset, and nothing created later (the service call for the reset stream, its
    // body) can bring it back.
    let mut served = serve(egress::STREAM_TABLE_CAP);
    served.open().await;
    let mut both = request(1, false);
    both.extend(frame(0x3, 0, 1, &[0, 0, 0, 8]));
    let mut write = served.drain_reads();
    write.write_all(&both).await.unwrap();
    timeout(Duration::from_secs(10), async {
        while served.counters.table_peak() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the request was never parsed");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(served.live.egress().stream_ids().is_empty());
    assert_eq!(served.counters.snapshot(), StallCuts::default());
    served.finish().await;
}

#[tokio::test]
async fn the_server_advertises_the_stream_limit_the_table_cap_is_derived_from() {
    // SETTINGS_MAX_CONCURRENT_STREAMS is identifier 3 (RFC 9113 section 6.5.2).
    let mut served = serve(egress::STREAM_TABLE_CAP);
    let settings = served.open().await;
    let limit = settings
        .iter()
        .find(|(id, _)| *id == 3)
        .map(|(_, value)| *value);
    assert_eq!(limit, Some(H2_MAX_CONCURRENT_STREAMS));
    served.finish().await;
}
