//! This module holds the per-connection table of live HTTP/2 client streams.
//!
//! The table holds one state machine per client stream id, created when the request's HEADERS
//! frame is parsed off the wire and removed only on wire evidence (a frame that ends the stream,
//! or a GOAWAY that leaves it unprocessed) or when the connection (and with it the table) ends.
//!
//! WHY a table at all: once a response body has ended on the server's side, hyper's stream task
//! completes and the body (with its lease and marker) is dropped, yet up to one chunk of DATA can
//! still be queued in h2 behind a stalled stream window: hyper hands a chunk over only while the
//! stream's send buffer holds less than the credit the peer has granted, so a body with more than
//! one chunk left parks its next chunk with the body still open, where the stream marker acts. The
//! stream holds a `max_concurrent_streams` slot and has no handle left to reset it, so the only
//! bound on it is cutting the connection, and only the wire says when it is really over. Counting
//! bodies ended against END_STREAM frames written is not an option: a peer reset after the body
//! ended writes no END_STREAM, and a tunnel's closing END_STREAM has no body, so the counts drift
//! apart and would cut healthy connections.
//!
//! A stream id has three states. [`StreamState::Open`] is created at parse time by
//! [`StreamTable::read_headers`], the only creator: neither the service call nor the response body
//! creates an entry, so a stream reset before the service saw it cannot gain one after its
//! terminal wire event.
//!
//! [`StreamState::Final`] is entered when [`StreamTable::register_final`] sees the body return the
//! frame that carries END_STREAM; registration only UPGRADES an existing entry. A `Final` entry is
//! never aged: while the last chunk waits at zero credit, the stream marker (see the `live`
//! module) cuts that one stream and not the connection.
//!
//! [`StreamState::Queued`] is entered when [`StreamTable::hand_over`] sees the body of a `Final`
//! entry dropped, which is hyper's hand-over of every byte to h2's send buffer (or the drop of a
//! cut stream). The entry ages unless the stream's own DATA progresses ([`StreamTable::scan`]).
//!
//! INVARIANT: Only `read_headers` inserts an entry, so a terminal event can never be undone by a
//! later registration or hand-over.
//!
//! INVARIANT: The cap fails closed: an insert beyond it inserts nothing and reports
//! [`ParseOutcome::Overflow`], because an entry count above what the h2 limits and the parse lag
//! allow means a parser or accounting defect or a peer outside the protocol, and a table that
//! merely stopped inserting would let that peer switch the liveness gate off.
//!
//! INVARIANT: Ids above the lowest GOAWAY last-stream-id accepted by the writer never get an
//! entry, and the entries already above it are dropped when that GOAWAY is written: h2 ignores
//! HEADERS above it and writes no RST_STREAM for them, so those entries would otherwise outlive
//! their stream. Only `Open` entries can be above it (nothing was served for them).
//!
//! INVARIANT: A stream's progress is its OWN written DATA payload. Another stream's DATA, a
//! tunnel's DATA, a PING or SETTINGS acknowledgement and any other connection write never refresh
//! an entry.

use std::collections::HashMap;
use std::task::Waker;
use std::time::Duration;

use tokio::time::Instant;

/// How much of a stream's queued DATA tail has been written, tracked from the start of the current
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TailProgress {
    /// DATA payload bytes the body handed to h2 for the stream.
    handed: u64,
    /// The stream's written DATA payload bytes when the current window began.
    origin_written: u64,
    /// The tail that remained unwritten when the current window began.
    origin_remaining: u64,
}

/// Where one stream is in its life, as far as the egress bounds care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamState {
    /// The request was parsed; the response is still being produced.
    Open,
    /// The response body returned its END_STREAM-bearing frame; hyper has not yet handed it over.
    Final,
    /// The body is gone; its tail may still be queued in h2. `since` starts the current window.
    Queued {
        /// When the current no-progress window began.
        since: Instant,
        /// The tail's progress over that window.
        tail: TailProgress,
    },
}

/// One stream id's state and the DATA payload bytes the writer has accepted for it.
#[derive(Debug)]
struct Entry {
    /// The state machine position.
    state: StreamState,
    /// DATA payload bytes the writer accepted for this stream, in every state.
    written: u64,
    /// The task parked by [`StreamTable::backlog`] until this stream's written bytes advance or its
    /// entry ends: the tunnel session waiting for its peer to take earlier frames.
    waker: Option<Waker>,
}

/// How far ahead of the wire a tunnel's application writes are, against a cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Backlog {
    /// The unwritten bytes are within the cap, or the stream has no entry any more.
    Within,
    /// The unwritten bytes exceed the cap; the caller's waker is parked on the entry.
    Over,
}

/// What parsing a request's HEADERS frame did to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseOutcome {
    /// No entry was created: a repeat HEADERS on a known stream, an even id, or an id above the
    /// final GOAWAY (h2 ignores those streams).
    Ignored,
    /// An `Open` entry was created and the table now holds `len` entries.
    Inserted {
        /// The table's length after the insert.
        len: usize,
        /// Whether this insert is the first to take the table past half its cap.
        past_half: bool,
    },
    /// The insert would exceed the cap. Nothing was inserted; the connection must be cut.
    Overflow,
}

/// A `Queued` entry whose own stream made no progress for the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EndedStall {
    /// How long the entry's current window had run without progress.
    pub(super) age: Duration,
    /// How many entries the table held.
    pub(super) entries: usize,
}

/// The per-connection stream table. See the module documentation.
#[derive(Debug)]
pub(super) struct StreamTable {
    /// Entries by client stream id.
    map: HashMap<u32, Entry>,
    /// The highest odd stream id whose request HEADERS were parsed.
    highest_client_sid: u32,
    /// The lowest last-stream-id of any GOAWAY the writer accepted.
    min_goaway: u32,
    /// The id of a HEADERS frame with END_STREAM whose END_HEADERS has not been written yet; the
    /// stream ends at the CONTINUATION that ends the header block.
    continuation_end_stream: Option<u32>,
    /// Set when an insert exceeded `cap`; the table inserts nothing afterwards.
    overflowed: bool,
    /// Whether the half-cap warning was already due.
    warned: bool,
    /// The most entries the table may hold.
    cap: usize,
}

impl StreamTable {
    /// An empty table that fails closed past `cap` entries.
    pub(super) fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            highest_client_sid: 0,
            min_goaway: 0x7fff_ffff,
            continuation_end_stream: None,
            overflowed: false,
            warned: false,
            cap,
        }
    }

    /// The read parser parsed a request HEADERS frame header for `sid`: creates its `Open` entry
    /// when `sid` is odd, above every id seen so far and not above the final GOAWAY's last stream
    /// id. Ids above that raise the highest-seen id only.
    pub(super) fn read_headers(&mut self, sid: u32) -> ParseOutcome {
        if sid.is_multiple_of(2) || sid <= self.highest_client_sid {
            return ParseOutcome::Ignored;
        }
        self.highest_client_sid = sid;
        if sid > self.min_goaway || self.overflowed {
            return ParseOutcome::Ignored;
        }
        if self.map.len() >= self.cap {
            self.overflowed = true;
            return ParseOutcome::Overflow;
        }
        self.map.insert(
            sid,
            Entry {
                state: StreamState::Open,
                written: 0,
                waker: None,
            },
        );
        let len = self.map.len();
        let past_half = !self.warned && len > self.cap / 2;
        self.warned |= past_half;
        ParseOutcome::Inserted { len, past_half }
    }

    /// Removes `sid`'s entry, returning the task parked on it so the caller can wake it once the
    /// table's lock is released.
    fn remove(&mut self, sid: u32) -> Option<Waker> {
        self.map.remove(&sid).and_then(|entry| entry.waker)
    }

    /// The read parser parsed a whole RST_STREAM frame for `sid`: the peer cancelled it. Returns
    /// the task parked on the entry, to be woken.
    pub(super) fn read_reset(&mut self, sid: u32) -> Option<Waker> {
        self.remove(sid)
    }

    /// The writer accepted `n` DATA payload bytes of `sid`. Returns the task parked on the entry
    /// (and clears it), to be woken: its backlog just shrank.
    pub(super) fn data_written(&mut self, sid: u32, n: u64) -> Option<Waker> {
        let entry = self.map.get_mut(&sid)?;
        entry.written = entry.written.saturating_add(n);
        entry.waker.take()
    }

    /// Whether `sent` bytes written by the application to `sid`'s tunnel are within `cap` bytes of
    /// what the wire has taken. Over the cap, `waker` is parked on the entry (replacing any earlier
    /// one) and the next [`StreamTable::data_written`] or removal of the entry returns it; the
    /// check and the parking are one step under the table's lock, so a write cannot slip between
    /// them. An absent entry is `Within`: the stream ended, and the caller's next write reports it.
    pub(super) fn backlog(&mut self, sid: u32, sent: u64, cap: u64, waker: &Waker) -> Backlog {
        let Some(entry) = self.map.get_mut(&sid) else {
            return Backlog::Within;
        };
        if sent.saturating_sub(entry.written) <= cap {
            entry.waker = None;
            return Backlog::Within;
        }
        match &mut entry.waker {
            Some(parked) if parked.will_wake(waker) => {}
            slot => *slot = Some(waker.clone()),
        }
        Backlog::Over
    }

    /// The writer accepted the last byte of a frame that ends `sid`: DATA or HEADERS with
    /// END_STREAM (HEADERS with END_HEADERS), or RST_STREAM. Returns the task parked on the entry,
    /// to be woken.
    pub(super) fn write_ended(&mut self, sid: u32) -> Option<Waker> {
        self.remove(sid)
    }

    /// The writer accepted the last byte of a HEADERS frame with END_STREAM but without
    /// END_HEADERS: the stream ends at the CONTINUATION that ends the block.
    pub(super) fn write_headers_continue(&mut self, sid: u32) {
        self.continuation_end_stream = Some(sid);
    }

    /// The writer accepted the last byte of a CONTINUATION frame with END_HEADERS. Returns the task
    /// parked on the entry it ended, to be woken.
    pub(super) fn write_continuation_end(&mut self, sid: u32) -> Option<Waker> {
        if self.continuation_end_stream == Some(sid) {
            self.continuation_end_stream = None;
            return self.remove(sid);
        }
        None
    }

    /// The writer accepted a GOAWAY whose last stream id is `last`: ids above it are never
    /// processed, so their entries (all `Open`) are dropped, and no entry above it is created
    /// later.
    pub(super) fn goaway(&mut self, last: u32) {
        self.min_goaway = self.min_goaway.min(last);
        self.map.retain(|sid, entry| {
            let beyond = *sid > last;
            debug_assert!(
                !beyond || entry.state == StreamState::Open,
                "a GOAWAY purge removed a stream past Open"
            );
            !beyond
        });
    }

    /// `Open -> Final` for `sid`: its body returned the frame carrying END_STREAM. Does nothing for
    /// an id with no entry (a wire event already ended the stream) or one past `Open`.
    pub(super) fn register_final(&mut self, sid: u32) {
        if let Some(entry) = self.map.get_mut(&sid) {
            if entry.state == StreamState::Open {
                entry.state = StreamState::Final;
            }
        }
    }

    /// `Final -> Queued` for `sid`: its body was dropped after handing `handed` DATA payload bytes
    /// to h2. An `Open` entry stays `Open` (the body was dropped before the final frame: a cut
    /// stream or a peer reset, which ends on the wire); an absent id stays absent.
    pub(super) fn hand_over(&mut self, sid: u32, handed: u64, now: Instant) {
        if let Some(entry) = self.map.get_mut(&sid) {
            if entry.state == StreamState::Final {
                entry.state = StreamState::Queued {
                    since: now,
                    tail: TailProgress {
                        handed,
                        origin_written: entry.written,
                        origin_remaining: handed.saturating_sub(entry.written),
                    },
                };
            }
        }
    }

    /// Looks for a `Queued` entry whose own stream wrote less than `min(floor, tail remaining at
    /// the window's start)` DATA payload bytes over a whole `window`, and returns the first it
    /// finds.
    ///
    /// An entry that reached its threshold starts a new window now, and nothing else refreshes it
    /// (only a stream's own written DATA is its progress). `floor` is the connection unit's own
    /// floor, so both units judge "too slow" by one number. A tail smaller than the floor that the
    /// reader drains within the window leaves at its END_STREAM write; one only partly drained when
    /// the window ends is cut. An entry with nothing left to write (only its END_STREAM frame is
    /// outstanding) is always at its threshold: it waits on the socket, which the connection unit
    /// bounds, not on the peer's credit.
    pub(super) fn scan(
        &mut self,
        now: Instant,
        window: Duration,
        floor: u64,
    ) -> Option<EndedStall> {
        let entries = self.map.len();
        let mut stalled = None;
        for entry in self.map.values_mut() {
            let StreamState::Queued { since, tail } = &mut entry.state else {
                continue;
            };
            let progressed = entry.written.saturating_sub(tail.origin_written);
            if progressed >= floor.min(tail.origin_remaining) {
                *since = now;
                tail.origin_written = entry.written;
                tail.origin_remaining = tail.handed.saturating_sub(entry.written);
                continue;
            }
            let age = now.saturating_duration_since(*since);
            if age >= window && stalled.is_none() {
                stalled = Some(EndedStall { age, entries });
            }
        }
        stalled
    }

    /// The entries by id and state, in id order, for tests.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Vec<(u32, StreamState)> {
        let mut all: Vec<_> = self.map.iter().map(|(sid, e)| (*sid, e.state)).collect();
        all.sort_by_key(|(sid, _)| *sid);
        all
    }

    /// The ids with an entry, in order, for tests.
    #[cfg(test)]
    pub(super) fn ids(&self) -> Vec<u32> {
        self.snapshot().into_iter().map(|(sid, _)| sid).collect()
    }

    /// How many entries the table holds, for tests.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    /// The DATA payload bytes written for `sid`, for tests.
    #[cfg(test)]
    pub(super) fn written(&self, sid: u32) -> Option<u64> {
        self.map.get(&sid).map(|e| e.written)
    }
}

#[cfg(test)]
mod tests;
