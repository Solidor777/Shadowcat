//! Egress no-progress accounting: what a cut is counted as, the counters the serve loop owns, and
//! the connection unit's evidence (how many response bytes the socket accepted, and whether its
//! writer is parked).
//!
//! One definition of "no progress" is shared by every unit that cuts a response: a peer that
//! returns no flow-control credit for a pending chunk (the stream unit), or a socket that accepts
//! nothing (the connection unit), for [`ConnLimits::egress_window`](super::ConnLimits). Each cut
//! is counted under exactly one [`StallUnit`], through [`StallCounters::record_stall`] and nothing
//! else, so the serve loop's closing report ([`DrainReport::stall_cuts`](super::DrainReport)) says
//! which bound fired.
//!
//! The connection unit measures the socket, not the application: [`WriteCounter`] sits in the
//! connection's IO and adds the response bytes the socket accepted to [`Egress`]; [`Sampler`], run
//! by `drive` on every tick, cuts a connection whose writer has been parked, with fewer than
//! `egress_floor` bytes accepted, for the whole window. On HTTP/2 only DATA frame payload counts: a
//! peer that keeps acknowledging PINGs and SETTINGS advertises liveness without draining, and raw
//! bytes would let those acknowledgements move the counter.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use tokio::time::Instant;

use super::StallCuts;

/// Which bound cut a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StallUnit {
    /// One HTTP/2 stream whose pending chunk had no send credit for the window.
    Stream,
    /// A whole connection whose socket accepted almost nothing for the window.
    Connection,
}

/// The serve loop's cut counters, shared by every connection it serves.
#[derive(Default)]
pub(super) struct StallCounters {
    /// Streams cut by the stream unit.
    stream: AtomicUsize,
    /// Connections cut by the connection unit.
    connection: AtomicUsize,
}

impl StallCounters {
    /// Counts one cut under `unit`. The only function that increments a counter.
    pub(super) fn record_stall(&self, unit: StallUnit) {
        let counter = match unit {
            StallUnit::Stream => &self.stream,
            StallUnit::Connection => &self.connection,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> StallCuts {
        StallCuts {
            stream: self.stream.load(Ordering::Relaxed),
            connection: self.connection.load(Ordering::Relaxed),
        }
    }
}

/// What the connection's writer has done, as the connection unit reads it.
#[derive(Default)]
pub(super) struct Egress {
    /// Response bytes the socket accepted. The TLS writer buffers up to 64 KiB (rustls's
    /// `DEFAULT_BUFFER_LIMIT`), so this runs ahead of what the peer has read. HTTP/2: DATA frame
    /// payload bytes; HTTP/1: raw bytes.
    data_accepted: AtomicU64,
    /// A LEVEL: the last write-side poll returned `Pending`. A writer that returns `Pending` is
    /// not polled again until its socket is writable, so an edge flag read once per tick would be
    /// true once and then false for as long as the writer stays parked; the level stays true.
    write_parked: AtomicBool,
}

impl Egress {
    /// Response bytes the socket has accepted so far.
    pub(super) fn accepted(&self) -> u64 {
        self.data_accepted.load(Ordering::Relaxed)
    }

    /// Whether the writer is parked.
    pub(super) fn parked(&self) -> bool {
        self.write_parked.load(Ordering::Relaxed)
    }
}

/// How the accepted bytes of one connection are counted.
enum Rule {
    /// HTTP/1: every accepted byte.
    Raw,
    /// HTTP/2: DATA frame payload bytes, found by walking the frames the writer emits.
    H2(FrameParser),
}

/// The connection's write side bookkeeping: counts what each write accepted into [`Egress`] and
/// keeps its parked level. Owned by the connection's IO, whose write methods call it.
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
                Rule::H2(FrameParser::default())
            } else {
                Rule::Raw
            },
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

    /// How many of the first `n` bytes of `bufs` count under this connection's rule.
    fn counted(&mut self, bufs: &[io::IoSlice<'_>], n: usize) -> u64 {
        match &mut self.rule {
            Rule::Raw => n as u64,
            Rule::H2(parser) => {
                let mut left = n;
                let mut data = 0;
                for buf in bufs {
                    if left == 0 {
                        break;
                    }
                    let take = left.min(buf.len());
                    data += parser.data_payload_in(&buf[..take]);
                    left -= take;
                }
                data
            }
        }
    }
}

/// The length of an HTTP/2 frame header (RFC 9113 section 4.1).
const FRAME_HEADER_LEN: usize = 9;

/// The type code of a DATA frame (RFC 9113 section 6.1).
const FRAME_TYPE_DATA: u8 = 0;

/// Walks the HTTP/2 frames a connection writes, in whatever pieces the writer accepts them, and
/// counts DATA payload bytes. O(frames): a payload is skipped by arithmetic, never inspected.
///
/// INVARIANT: the first byte fed is the first byte of a frame. The server's first write is its
/// SETTINGS frame, and the parser sits after the TLS layer, so it sees plaintext frames.
///
/// Counts a PADDED DATA frame's whole length field: the server writes no padding, and a padded
/// frame would be overcounted by at most 255 bytes.
#[derive(Default)]
pub(super) struct FrameParser {
    /// The frame header bytes collected so far.
    header: [u8; FRAME_HEADER_LEN],
    /// How many `header` bytes are filled, `0..FRAME_HEADER_LEN`.
    filled: usize,
    /// Payload bytes of the current frame still to skip.
    left: u32,
    /// Whether the current frame is DATA.
    is_data: bool,
    /// Loop passes taken, so a test can assert the cost is per frame and not per byte.
    #[cfg(test)]
    steps: usize,
}

impl FrameParser {
    /// Consumes `bytes`, the next bytes the writer accepted, and returns how many were DATA
    /// payload.
    pub(super) fn data_payload_in(&mut self, mut bytes: &[u8]) -> u64 {
        let mut data = 0u64;
        while !bytes.is_empty() {
            #[cfg(test)]
            {
                self.steps += 1;
            }
            if self.left > 0 {
                let skip = bytes.len().min(self.left as usize);
                if self.is_data {
                    data += skip as u64;
                }
                self.left -= skip as u32;
                bytes = &bytes[skip..];
                continue;
            }
            let take = (FRAME_HEADER_LEN - self.filled).min(bytes.len());
            self.header[self.filled..self.filled + take].copy_from_slice(&bytes[..take]);
            self.filled += take;
            bytes = &bytes[take..];
            if self.filled == FRAME_HEADER_LEN {
                self.left = u32::from_be_bytes([0, self.header[0], self.header[1], self.header[2]]);
                self.is_data = self.header[3] == FRAME_TYPE_DATA;
                self.filled = 0;
            }
        }
        data
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
