//! Egress no-progress accounting: what a cut is counted as, and the counters the serve loop owns.
//!
//! One definition of "no progress" is shared by every unit that cuts a response: a peer that
//! returns no flow-control credit for a pending chunk (the stream unit), or a socket that accepts
//! nothing (the connection unit), for [`ConnLimits::egress_window`](super::ConnLimits). Each cut
//! is counted under exactly one [`StallUnit`], through [`StallCounters::record_stall`] and nothing
//! else, so the serve loop's closing report ([`DrainReport::stall_cuts`](super::DrainReport)) says
//! which bound fired.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::StallCuts;

/// Which bound cut a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StallUnit {
    /// One HTTP/2 stream whose pending chunk had no send credit for the window.
    Stream,
}

/// The serve loop's cut counters, shared by every connection it serves.
#[derive(Default)]
pub(super) struct StallCounters {
    /// Streams cut by the stream unit.
    stream: AtomicUsize,
}

impl StallCounters {
    /// Counts one cut under `unit`. The only function that increments a counter.
    pub(super) fn record_stall(&self, unit: StallUnit) {
        let counter = match unit {
            StallUnit::Stream => &self.stream,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// The counts so far.
    pub(super) fn snapshot(&self) -> StallCuts {
        StallCuts {
            stream: self.stream.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests;
