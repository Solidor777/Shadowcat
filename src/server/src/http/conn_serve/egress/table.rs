//! The per-connection table of live HTTP/2 client streams: one entry per client stream id, created
//! when the request's HEADERS frame is parsed off the wire and removed only on wire evidence that
//! the stream ended, or when the connection (and with it the table) ends.
//!
//! WHY the table is fed from the wire and not from the request path: once a response body has
//! ended on the server's side, hyper's stream task completes and the body is dropped, yet DATA can
//! still be queued in h2 behind a stalled stream window. The stream holds a
//! `max_concurrent_streams` slot and has no handle left to reset it, so only the wire says when it
//! is really over. Counting bodies ended against END_STREAM frames written is not an option: a peer
//! reset after the body ended writes no END_STREAM, and a tunnel's closing END_STREAM has no body,
//! so the counts drift apart.
//!
//! INVARIANTS:
//! 1. [`StreamTable::read_headers`] is the only creator of an entry. Not the service call, not the
//!    response body: a stream reset before the service saw it must never gain an entry after its
//!    terminal wire event. Every other operation is a no-op for an id with no entry, so a terminal
//!    event can never be undone by a later one.
//! 2. The cap fails closed: an insert beyond it inserts nothing and reports
//!    [`ParseOutcome::Overflow`], because an entry count above what the h2 limits and the parse lag
//!    allow means a parser or accounting defect or a peer outside the protocol, and a table that
//!    merely stopped inserting would let that peer switch the liveness gate off.
//! 3. Ids above the lowest GOAWAY last-stream-id accepted by the writer never get an entry, and the
//!    entries already above it are dropped when that GOAWAY is written: h2 ignores HEADERS above it
//!    and writes no RST_STREAM for them, so those entries would otherwise outlive their stream.

use std::collections::HashSet;

/// What parsing a request's HEADERS frame did to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseOutcome {
    /// No entry was created: a repeat HEADERS on a known stream, an even id, or an id above the
    /// final GOAWAY (h2 ignores those streams).
    Ignored,
    /// An entry was created and the table now holds `len` entries.
    Inserted {
        /// The table's length after the insert.
        len: usize,
        /// Whether this insert is the first to take the table past half its cap.
        past_half: bool,
    },
    /// The insert would exceed the cap. Nothing was inserted; the connection must be cut.
    Overflow,
}

/// The per-connection stream table. See the module documentation.
#[derive(Debug)]
pub(super) struct StreamTable {
    /// The client stream ids with an entry.
    open: HashSet<u32>,
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
            open: HashSet::new(),
            highest_client_sid: 0,
            min_goaway: 0x7fff_ffff,
            continuation_end_stream: None,
            overflowed: false,
            warned: false,
            cap,
        }
    }

    /// The read parser parsed a request HEADERS frame header for `sid`: creates its entry when
    /// `sid` is odd, above every id seen so far and not above the final GOAWAY's last stream id.
    /// Ids above that raise the highest-seen id only.
    pub(super) fn read_headers(&mut self, sid: u32) -> ParseOutcome {
        if sid.is_multiple_of(2) || sid <= self.highest_client_sid {
            return ParseOutcome::Ignored;
        }
        self.highest_client_sid = sid;
        if sid > self.min_goaway || self.overflowed {
            return ParseOutcome::Ignored;
        }
        if self.open.len() >= self.cap {
            self.overflowed = true;
            return ParseOutcome::Overflow;
        }
        self.open.insert(sid);
        let len = self.open.len();
        let past_half = !self.warned && len > self.cap / 2;
        self.warned |= past_half;
        ParseOutcome::Inserted { len, past_half }
    }

    /// The read parser parsed a whole RST_STREAM frame for `sid`: the peer cancelled it.
    pub(super) fn read_reset(&mut self, sid: u32) {
        self.open.remove(&sid);
    }

    /// The writer accepted the last byte of a frame that ends `sid`: DATA or HEADERS with
    /// END_STREAM (HEADERS with END_HEADERS), or RST_STREAM.
    pub(super) fn write_ended(&mut self, sid: u32) {
        self.open.remove(&sid);
    }

    /// The writer accepted the last byte of a HEADERS frame with END_STREAM but without
    /// END_HEADERS: the stream ends at the CONTINUATION that ends the block.
    pub(super) fn write_headers_continue(&mut self, sid: u32) {
        self.continuation_end_stream = Some(sid);
    }

    /// The writer accepted the last byte of a CONTINUATION frame with END_HEADERS.
    pub(super) fn write_continuation_end(&mut self, sid: u32) {
        if self.continuation_end_stream == Some(sid) {
            self.continuation_end_stream = None;
            self.open.remove(&sid);
        }
    }

    /// The writer accepted a GOAWAY whose last stream id is `last`: ids above it are never
    /// processed, so their entries are dropped, and no entry above it is created later.
    pub(super) fn goaway(&mut self, last: u32) {
        self.min_goaway = self.min_goaway.min(last);
        self.open.retain(|sid| *sid <= last);
    }

    /// The ids with an entry, in order, for tests.
    #[cfg(test)]
    pub(super) fn ids(&self) -> Vec<u32> {
        let mut all: Vec<u32> = self.open.iter().copied().collect();
        all.sort_unstable();
        all
    }

    /// How many entries the table holds, for tests.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.open.len()
    }
}

#[cfg(test)]
mod tests;
