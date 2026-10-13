//! This module accounts for the egress no-progress bounds.
//!
//! It covers what a cut is counted as, the counters the serve loop owns, the connection unit's
//! evidence (how many response bytes the socket accepted, and whether its writer is parked), and
//! the wire-fed table of live HTTP/2 streams.
//!
//! One definition of "no progress" is shared by the units that cut a response for it: a peer that
//! returns no flow-control credit for a pending chunk (the stream unit), a socket that accepts
//! fewer than `egress_floor` bytes while its writer is parked (the connection unit), or an ended
//! response whose own stream's queued tail makes less than `min(egress_floor, remaining)` bytes of
//! progress (the ended-unflushed unit, tracked by the `table` module), for
//! [`ConnLimits::egress_window`](super::ConnLimits). Each cut is counted under exactly one
//! [`StallUnit`], through [`StallCounters::record_stall`] and nothing else, so the serve loop's
//! closing report ([`DrainReport::stall_cuts`](super::DrainReport)) says which bound fired.
//!
//! The connection unit measures the socket, not the application: [`WriteCounter`] sits in the
//! connection's IO and adds the response bytes the socket accepted to [`Egress`]; [`Sampler`], run
//! by `drive` on every tick, cuts a connection whose writer has been parked, with fewer than
//! `egress_floor` bytes accepted, for the whole window. On HTTP/2 only DATA frame payload counts: a
//! peer that keeps acknowledging PINGs and SETTINGS advertises liveness without draining, and raw
//! bytes would let those acknowledgements move the counter.
//!
//! On HTTP/2 the connection's IO also walks the frames of both directions with the `parser` module
//! ([`WriteCounter`] for what the writer accepts, [`ReadObserver`] for what the codec reads) and
//! keeps the `table` module's per-stream table in step with the wire. The table fails closed: a
//! connection whose table outgrows its cap is cut and counted as [`StallUnit::TrackerOverflow`].
//! Response bodies add the other half of each stream's life (the frame that carries END_STREAM, and
//! the body's drop), and [`Egress::cut_ended_unflushed`], run by `drive` on each tick at which the
//! writer is not parked, cuts a connection whose finished response's own queued tail made no
//! progress for the window.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tokio::time::Instant;

use self::parser::{
    FrameEvent, FrameParser, FLAG_END_HEADERS, FLAG_END_STREAM, FRAME_TYPE_CONTINUATION,
    FRAME_TYPE_DATA, FRAME_TYPE_HEADERS, FRAME_TYPE_RST_STREAM,
};
#[cfg(test)]
use self::table::StreamState;
use self::table::{Backlog, ParseOutcome, StreamTable};
use super::kill::{ConnKill, KillReason};
use super::{StallCuts, H2_MAX_CONCURRENT_STREAMS, H2_MAX_FRAME_BYTES};

mod parser;
mod table;

/// The length of an HTTP/2 frame header, the smallest frame (RFC 9113 section 4.1).
pub(super) const FRAME_HEADER_BYTES: usize = parser::FRAME_HEADER_LEN;

/// The initial capacity of the codec's framed reader buffer: the allocation
/// `tokio_util::codec::FramedRead` makes before it has read anything.
pub(super) const CODEC_INITIAL_READ_BYTES: usize = 8 * 1024;

/// The most bytes one read of the codec's framed reader returns, as the stream table's lag test
/// asserts it: no read exceeds this bound, and a read made after a full-size frame's header exceeds
/// [`CODEC_INITIAL_READ_BYTES`]. The value is the larger of [`H2_MAX_FRAME_BYTES`] (the receive
/// limit the connection advertises) and [`CODEC_INITIAL_READ_BYTES`].
pub(super) const CODEC_READ_MAX_BYTES: usize =
    if H2_MAX_FRAME_BYTES as usize > CODEC_INITIAL_READ_BYTES {
        H2_MAX_FRAME_BYTES as usize
    } else {
        CODEC_INITIAL_READ_BYTES
    };

/// How many entries the table may hold above `max_concurrent_streams` because the read parser sees
/// bytes the codec has read but h2 has not processed, and h2's refusal RST_STREAM frames are queued
/// but not yet written.
///
/// Derivation: one read of the codec's reader carries at most [`CODEC_READ_MAX_BYTES`], and the
/// smallest parseable HEADERS frame is [`FRAME_HEADER_BYTES`] (a frame header with an empty block),
/// so one read can add `CODEC_READ_MAX_BYTES / FRAME_HEADER_BYTES` entries. At most two reads are in
/// flight at once (the one being processed and the one buffered), and the refusals still queued add
/// at most about as many again; five reads' worth carries a margin over that. The minimum-frame
/// integration test asserts the premises (read size, frame size) before it asserts the peak against
/// this derivation, after a full-size frame as well as after ordinary reads.
pub(super) const PARSE_LAG_SLACK: usize = 5 * (CODEC_READ_MAX_BYTES / FRAME_HEADER_BYTES);

/// The most entries a connection's stream table may hold: the streams h2 admits plus the parse lag
/// the table tolerates ([`PARSE_LAG_SLACK`]).
pub(super) const STREAM_TABLE_CAP: usize = H2_MAX_CONCURRENT_STREAMS as usize + PARSE_LAG_SLACK;

/// Which bound cut a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StallUnit {
    /// One HTTP/2 stream whose pending chunk had no send credit for the window.
    Stream,
    /// A whole connection whose socket accepted almost nothing for the window.
    Connection,
    /// A connection whose ended response's own queued tail made no progress for the window.
    EndedUnflushed,
    /// A connection whose stream table outgrew its cap.
    TrackerOverflow,
}

/// The serve loop's cut counters, shared by every connection it serves.
#[derive(Default)]
pub(super) struct StallCounters {
    /// Streams cut by the stream unit.
    stream: AtomicUsize,
    /// Connections cut by the connection unit.
    connection: AtomicUsize,
    /// Connections cut for an ended response whose tail made no progress.
    ended_unflushed: AtomicUsize,
    /// Connections cut because their stream table outgrew its cap.
    tracker_overflow: AtomicUsize,
    /// The most entries any connection's stream table held.
    table_peak: AtomicUsize,
}

impl StallCounters {
    /// Counts one cut under `unit`. The only function that increments a counter.
    pub(super) fn record_stall(&self, unit: StallUnit) {
        let counter = match unit {
            StallUnit::Stream => &self.stream,
            StallUnit::Connection => &self.connection,
            StallUnit::EndedUnflushed => &self.ended_unflushed,
            StallUnit::TrackerOverflow => &self.tracker_overflow,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Raises the table-length peak to `len` when it is higher.
    fn note_table_len(&self, len: usize) {
        self.table_peak.fetch_max(len, Ordering::Relaxed);
    }

    /// The most entries any connection's stream table held.
    pub(super) fn table_peak(&self) -> usize {
        self.table_peak.load(Ordering::Relaxed)
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> StallCuts {
        StallCuts {
            stream: self.stream.load(Ordering::Relaxed),
            connection: self.connection.load(Ordering::Relaxed),
            ended_unflushed: self.ended_unflushed.load(Ordering::Relaxed),
            tracker_overflow: self.tracker_overflow.load(Ordering::Relaxed),
        }
    }
}

/// Locks `m`, ignoring poison: the state it guards is always consistent between statements.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wakes a task a table operation handed back. Called after the table's lock is released.
fn wake(parked: Option<Waker>) {
    if let Some(waker) = parked {
        waker.wake();
    }
}

/// What a connection's writer and its streams have done, as the egress units read it: the
/// connection unit's evidence and the table of live HTTP/2 streams.
pub(super) struct Egress {
    /// Response bytes the socket accepted. The TLS writer buffers up to 64 KiB (rustls's
    /// `DEFAULT_BUFFER_LIMIT`), so this runs ahead of what the peer has read. HTTP/2: DATA frame
    /// payload bytes; HTTP/1: raw bytes.
    data_accepted: AtomicU64,
    /// A LEVEL: the last write-side poll returned `Pending`. A writer that returns `Pending` is
    /// not polled again until its socket is writable, so an edge flag read once per tick would be
    /// true once and then false for as long as the writer stays parked; the level stays true.
    write_parked: AtomicBool,
    /// The wire-fed table of live HTTP/2 client streams (never filled on HTTP/1).
    streams: Mutex<StreamTable>,
    /// The serve loop's cut counters, which every cut site reaches through
    /// [`Egress::record_stall`].
    stalls: Arc<StallCounters>,
}

#[cfg(test)]
impl Default for Egress {
    /// A record with its own counters and the production table cap.
    fn default() -> Self {
        Self::new(Arc::new(StallCounters::default()), STREAM_TABLE_CAP)
    }
}

impl Egress {
    /// An empty record whose stream table holds at most `table_cap` entries and whose cuts count in
    /// `stalls`.
    pub(super) fn new(stalls: Arc<StallCounters>, table_cap: usize) -> Self {
        Self {
            data_accepted: AtomicU64::new(0),
            write_parked: AtomicBool::new(false),
            streams: Mutex::new(StreamTable::new(table_cap)),
            stalls,
        }
    }

    /// Counts one cut under `unit` in the serve loop's counters.
    pub(super) fn record_stall(&self, unit: StallUnit) {
        self.stalls.record_stall(unit);
    }

    /// The serve loop's counters, for tests.
    #[cfg(test)]
    pub(super) fn counters(&self) -> &StallCounters {
        &self.stalls
    }

    /// The stream ids with a table entry, in order, for tests.
    #[cfg(test)]
    pub(super) fn stream_ids(&self) -> Vec<u32> {
        lock(&self.streams).ids()
    }

    /// The DATA payload bytes the writer accepted for stream `sid`, for tests.
    #[cfg(test)]
    pub(super) fn stream_written(&self, sid: u32) -> Option<u64> {
        lock(&self.streams).written(sid)
    }

    /// Each table entry's id and phase (`open`, `final` or `queued`), in id order, for tests.
    #[cfg(test)]
    pub(super) fn stream_phases(&self) -> Vec<(u32, &'static str)> {
        lock(&self.streams)
            .snapshot()
            .into_iter()
            .map(|(sid, state)| {
                let phase = match state {
                    StreamState::Open => "open",
                    StreamState::Final => "final",
                    StreamState::Queued { .. } => "queued",
                };
                (sid, phase)
            })
            .collect()
    }

    /// Response bytes the socket has accepted so far.
    pub(super) fn accepted(&self) -> u64 {
        self.data_accepted.load(Ordering::Relaxed)
    }

    /// Whether the writer is parked.
    pub(super) fn parked(&self) -> bool {
        self.write_parked.load(Ordering::Relaxed)
    }

    /// The response body of stream `sid` returned the frame carrying END_STREAM.
    pub(super) fn register_final(&self, sid: u32) {
        lock(&self.streams).register_final(sid);
    }

    /// The response body of stream `sid` was dropped after handing `handed` DATA payload bytes to
    /// h2.
    pub(super) fn hand_over(&self, sid: u32, handed: u64) {
        lock(&self.streams).hand_over(sid, handed, Instant::now());
    }

    /// Cuts the connection when an ended response's own queued tail made no progress for `window`
    /// (progress is judged against `floor` bytes), counting it as one
    /// [`StallUnit::EndedUnflushed`] cut. Returns whether it cut: counted only when this call
    /// ended the connection, so a drain deadline or another unit that fired first is not
    /// double-counted.
    pub(super) fn cut_ended_unflushed(
        &self,
        kill: &ConnKill,
        now: Instant,
        window: Duration,
        floor: u64,
    ) -> bool {
        let Some(stall) = lock(&self.streams).scan(now, window, floor) else {
            return false;
        };
        if !kill.fire(KillReason::Stall) {
            return false;
        }
        tracing::warn!(
            unit = "ended_unflushed",
            age_ms = u64::try_from(stall.age.as_millis()).unwrap_or(u64::MAX),
            entries = stall.entries,
            "connection cut: a finished response's queued tail made no progress"
        );
        self.record_stall(StallUnit::EndedUnflushed);
        true
    }

    /// The read parser parsed a request HEADERS frame header for `sid`. Creates the stream's entry
    /// and records the table's length; when the table would outgrow its cap the connection is cut
    /// through `kill` and counted as one [`StallUnit::TrackerOverflow`] cut (only when this call
    /// ended the connection, so a drain that fired first is not double-counted).
    fn request_started(&self, sid: u32, kill: &ConnKill) {
        let outcome = lock(&self.streams).read_headers(sid);
        match outcome {
            ParseOutcome::Ignored => {}
            ParseOutcome::Inserted { len, past_half } => {
                self.stalls.note_table_len(len);
                if past_half {
                    tracing::warn!(
                        entries = len,
                        cap = STREAM_TABLE_CAP,
                        "a connection's stream table is past half its cap"
                    );
                }
            }
            ParseOutcome::Overflow => {
                if kill.fire(KillReason::Stall) {
                    tracing::warn!(
                        unit = "tracker_overflow",
                        cap = STREAM_TABLE_CAP,
                        "connection cut: its stream table outgrew its cap"
                    );
                    self.record_stall(StallUnit::TrackerOverflow);
                }
            }
        }
    }

    /// The writer accepted the last byte of a frame of `kind` with `flags` on `sid`.
    fn frame_written(&self, kind: u8, flags: u8, sid: u32) {
        let ends_stream = flags & FLAG_END_STREAM != 0;
        let ends_headers = flags & FLAG_END_HEADERS != 0;
        let parked = match kind {
            FRAME_TYPE_HEADERS if ends_stream && ends_headers => {
                lock(&self.streams).write_ended(sid)
            }
            FRAME_TYPE_HEADERS if ends_stream => {
                lock(&self.streams).write_headers_continue(sid);
                None
            }
            FRAME_TYPE_CONTINUATION if ends_headers => {
                lock(&self.streams).write_continuation_end(sid)
            }
            FRAME_TYPE_DATA if ends_stream => lock(&self.streams).write_ended(sid),
            FRAME_TYPE_RST_STREAM => lock(&self.streams).write_ended(sid),
            _ => None,
        };
        wake(parked);
    }

    /// The writer accepted `bytes` DATA payload bytes of stream `sid`; wakes a tunnel session
    /// parked on the stream's backlog.
    fn data_written(&self, sid: u32, bytes: u64) {
        let parked = lock(&self.streams).data_written(sid, bytes);
        wake(parked);
    }

    /// Whether the application has written no more than `cap` bytes to stream `sid`'s tunnel beyond
    /// what the writer has put on the wire (`sent` is the application's own running total of
    /// payload bytes). `Pending` parks `cx`'s task until the stream's written bytes advance or its
    /// entry ends.
    fn tunnel_within(&self, sid: u32, sent: u64, cap: u64, cx: &Context<'_>) -> Poll<()> {
        match lock(&self.streams).backlog(sid, sent, cap, cx.waker()) {
            Backlog::Within => Poll::Ready(()),
            Backlog::Over => Poll::Pending,
        }
    }

    /// The writer accepted a whole GOAWAY frame with last stream id `last`.
    fn goaway_written(&self, last: u32) {
        lock(&self.streams).goaway(last);
    }
}

/// A WebSocket session's view of its own HTTP/2 stream's progress on the wire: how many DATA
/// payload bytes the connection's writer has accepted for the tunnel's stream. The connection's
/// lease service puts one in the extensions of every HTTP/2 request.
///
/// WHY it exists: hyper's tunnel task passes every written chunk on to h2 whether or not the peer
/// has granted send credit, so a tunnel whose peer returns none absorbs writes without bound and
/// the sink writing to it never goes pending. The session compares its own written total with the
/// stream's written total and stops writing when it is further ahead than a send buffer.
///
/// # Examples
///
/// ```
/// use axum::extract::Request;
/// use shadowcat::http::conn_serve::TunnelProbe;
///
/// // Absent on HTTP/1 and when the router is served without `serve_router`.
/// let req = Request::new(axum::body::Body::empty());
/// assert!(req.extensions().get::<TunnelProbe>().is_none());
/// ```
#[derive(Clone)]
pub struct TunnelProbe {
    /// The connection's record, whose stream table counts the stream's written bytes.
    egress: Arc<Egress>,
    /// The request's HTTP/2 stream id.
    sid: u32,
}

impl std::fmt::Debug for TunnelProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelProbe")
            .field("sid", &self.sid)
            .finish_non_exhaustive()
    }
}

impl TunnelProbe {
    /// A probe of stream `sid` on the connection `egress` describes.
    pub(super) fn new(egress: Arc<Egress>, sid: u32) -> Self {
        Self { egress, sid }
    }

    /// `Ready` while the session has written no more than `cap` payload bytes beyond what the
    /// writer has put on the wire for the stream (`sent` is the session's own running total).
    /// `Pending` parks `cx`'s task until the stream's written bytes advance or its entry ends.
    pub(crate) fn poll_within(&self, cx: &Context<'_>, sent: u64, cap: u64) -> Poll<()> {
        self.egress.tunnel_within(self.sid, sent, cap, cx)
    }

    /// Reports `bytes` DATA payload bytes of the stream accepted by the connection's writer, as the
    /// write path would.
    #[cfg(test)]
    pub(crate) fn written_on_wire(&self, bytes: u64) {
        self.egress.data_written(self.sid, bytes);
    }

    /// Reports the end of the stream on the wire (a peer reset), as the read path would.
    #[cfg(test)]
    pub(crate) fn ended_on_wire(&self) {
        self.egress
            .frame_written(FRAME_TYPE_RST_STREAM, 0, self.sid);
    }

    /// A probe of stream `sid` on a fresh record whose stream table holds that stream.
    #[cfg(test)]
    pub(crate) fn detached(sid: u32) -> Self {
        let egress = Arc::new(Egress::default());
        let kill = ConnKill::new();
        egress.request_started(sid, &kill);
        Self::new(egress, sid)
    }
}

/// How the accepted bytes of one connection are counted.
enum Rule {
    /// HTTP/1: every accepted byte.
    Raw,
    /// HTTP/2: DATA frame payload bytes, found by walking the frames the writer emits.
    H2(FrameParser),
}

/// The connection's write side bookkeeping: counts what each write accepted into [`Egress`], keeps
/// its parked level, and on HTTP/2 keeps the stream table in step with the frames written. Owned by
/// the connection's IO, whose write methods call it.
pub(super) struct WriteCounter {
    /// Where the counts go.
    egress: Arc<Egress>,
    /// How accepted bytes are counted: chosen by `ServeIo::is_h2`, never inferred from ALPN text.
    rule: Rule,
}

impl WriteCounter {
    /// A counter feeding `egress`, counting HTTP/2 DATA payload when `is_h2`, raw bytes otherwise.
    pub(super) fn new(egress: Arc<Egress>, is_h2: bool) -> Self {
        Self {
            egress,
            rule: if is_h2 {
                Rule::H2(FrameParser::new())
            } else {
                Rule::Raw
            },
        }
    }

    /// The read-side observer of the same connection: it parses requests only on HTTP/2.
    pub(super) fn read_observer(&self) -> ReadObserver {
        ReadObserver {
            egress: self.egress.clone(),
            parser: matches!(self.rule, Rule::H2(_)).then(FrameParser::with_preface),
        }
    }

    /// Records the outcome of a `poll_write` or `poll_write_vectored` of `bufs`: `Pending` parks
    /// the writer, an accepted write unparks it and counts exactly the `n` bytes accepted (never
    /// the bytes offered: a vectored write may stop mid-slice).
    pub(super) fn after_write(
        &mut self,
        result: &Poll<io::Result<usize>>,
        bufs: &[io::IoSlice<'_>],
    ) {
        match result {
            Poll::Pending => self.egress.write_parked.store(true, Ordering::Relaxed),
            Poll::Ready(Ok(n)) => {
                self.egress.write_parked.store(false, Ordering::Relaxed);
                let counted = self.counted(bufs, *n);
                self.egress
                    .data_accepted
                    .fetch_add(counted, Ordering::Relaxed);
            }
            Poll::Ready(Err(_)) => {}
        }
    }

    /// Records the outcome of a `poll_flush` or `poll_shutdown`: `Pending` parks the writer, a
    /// completed one unparks it.
    pub(super) fn after_flush(&self, result: &Poll<io::Result<()>>) {
        match result {
            Poll::Pending => self.egress.write_parked.store(true, Ordering::Relaxed),
            Poll::Ready(Ok(())) => self.egress.write_parked.store(false, Ordering::Relaxed),
            Poll::Ready(Err(_)) => {}
        }
    }

    /// How many of the first `n` bytes of `bufs` count under this connection's rule. On HTTP/2 it
    /// also reports each frame the bytes completed to the stream table.
    fn counted(&mut self, bufs: &[io::IoSlice<'_>], n: usize) -> u64 {
        match &mut self.rule {
            Rule::Raw => n as u64,
            Rule::H2(parser) => {
                let egress = &self.egress;
                let mut data = 0;
                let mut on = |event: FrameEvent| match event {
                    FrameEvent::Data { sid, bytes } => {
                        data += bytes;
                        egress.data_written(sid, bytes);
                    }
                    FrameEvent::End { kind, flags, sid } => egress.frame_written(kind, flags, sid),
                    FrameEvent::GoAway(last) => egress.goaway_written(last),
                    FrameEvent::Start { .. } => {}
                };
                let mut left = n;
                for buf in bufs {
                    if left == 0 {
                        break;
                    }
                    let take = left.min(buf.len());
                    parser.feed(&buf[..take], &mut on);
                    left -= take;
                }
                data
            }
        }
    }
}

/// The connection's read side bookkeeping: on HTTP/2, walks the bytes the codec is handed and
/// reports each request HEADERS frame and each peer RST_STREAM to the stream table. Owned by the
/// connection's IO, whose read method calls it.
pub(super) struct ReadObserver {
    /// The connection's record, whose table the events feed.
    egress: Arc<Egress>,
    /// The frame walker, starting in the client preface; `None` on HTTP/1.
    parser: Option<FrameParser>,
}

impl ReadObserver {
    /// Walks `bytes`, which the codec is about to read. A table overflow cuts the connection
    /// through `kill`.
    pub(super) fn after_read(&mut self, bytes: &[u8], kill: &ConnKill) {
        let Some(parser) = &mut self.parser else {
            return;
        };
        let egress = &self.egress;
        parser.feed(bytes, &mut |event| match event {
            FrameEvent::Start {
                kind: FRAME_TYPE_HEADERS,
                sid,
            } => egress.request_started(sid, kill),
            FrameEvent::End {
                kind: FRAME_TYPE_RST_STREAM,
                sid,
                ..
            } => {
                let parked = lock(&egress.streams).read_reset(sid);
                wake(parked);
            }
            _ => {}
        });
    }
}

/// One observation of the connection's writer.
struct Sample {
    /// When it was taken.
    at: Instant,
    /// `Egress::accepted` at that instant.
    accepted: u64,
    /// Whether the writer was parked at that instant.
    parked: bool,
}

/// What a connection that tripped the sampler looked like, for the log line.
pub(super) struct Stall {
    /// The time the evidence spans.
    pub(super) span: Duration,
    /// The bytes the socket accepted over it.
    pub(super) accepted: u64,
}

/// A sliding window over the writer's observations. It fires when the ring is full, every sample
/// saw the writer parked, the TIMESTAMPED span from the oldest to the newest sample is at least
/// the window, and fewer than `floor` bytes were accepted over it.
///
/// Why sliding and not a fixed window: a stop that begins mid-window after progress is detected
/// within one tick of the window elapsing, not up to a whole window later. Why the span is read
/// from timestamps: a runtime stall that bunches ticks must not fill the ring in microseconds and
/// fire early. One equality boundary on both units: exactly `floor` bytes is progress.
pub(super) struct Sampler {
    /// The most recent samples, oldest first, at most `capacity`.
    ring: VecDeque<Sample>,
    /// `window / tick + 1` samples span one window; never fewer than two, so there is a span at
    /// all.
    capacity: usize,
    /// How long no progress is tolerated.
    window: Duration,
    /// Bytes that count as progress over a window.
    floor: u64,
}

impl Sampler {
    /// A sampler for `window` observed every `tick` (the period `drive` ticks at; nonzero), with
    /// `floor` bytes per window counting as progress.
    pub(super) fn new(window: Duration, tick: Duration, floor: u64) -> Self {
        let per_window =
            usize::try_from(window.as_nanos() / tick.as_nanos().max(1)).unwrap_or(usize::MAX - 1);
        let capacity = per_window.saturating_add(1).max(2);
        Self {
            ring: VecDeque::with_capacity(capacity.min(64)),
            capacity,
            window,
            floor,
        }
    }

    /// Records one observation and returns the evidence when it completes a stall.
    pub(super) fn observe(&mut self, at: Instant, accepted: u64, parked: bool) -> Option<Stall> {
        if self.ring.len() == self.capacity {
            self.ring.pop_front();
        }
        self.ring.push_back(Sample {
            at,
            accepted,
            parked,
        });
        if self.ring.len() < self.capacity || !self.ring.iter().all(|s| s.parked) {
            return None;
        }
        let (oldest, newest) = (self.ring.front()?, self.ring.back()?);
        let span = newest.at.saturating_duration_since(oldest.at);
        let delta = newest.accepted.saturating_sub(oldest.accepted);
        (span >= self.window && delta < self.floor).then_some(Stall {
            span,
            accepted: delta,
        })
    }
}

#[cfg(test)]
mod tests;
