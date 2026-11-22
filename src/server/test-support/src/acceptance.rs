//! The loopback premise of a keeps-reading control, stated once.
//!
//! The connection unit of the egress bounds judges the bytes the server's socket ACCEPTS, and over
//! loopback the socket accepts them in lumps: 100 to 260 KB on Windows (the peer's receive-window
//! step, plus a whole queued write accepted at once) and 0.9 to 1.1 MB on Linux (the autotuned
//! send buffer; the `egress_measure` program's `h1-progress` and `socket-grants` subcommands print
//! the lumps). A reader taking fewer bytes per window than a lump holds sees no acceptance for
//! whole windows and is cut, at every window length measured (1, 3, 10 and 60 s), because the lump
//! is a count of bytes and the floor is a count of bytes per window. A control that reads at a
//! rate near one lump per window therefore measures the loopback, not the unit.
//!
//! A control that expects NOT to be cut reads [`progress_bytes_per_window`] per window, about
//! eight Linux lumps, so a lump lands every eighth of a window, and asserts that premise in the
//! same test through [`Acceptances::assert_premise`]. The multiple states a property of the
//! loopback and the measure, not a promise of the unit at lower rates.

use std::time::Duration;

use shadowcat::http::conn_serve::ConnLimits;
use tokio::time::Instant;

/// How many multiples of `ConnLimits::egress_floor` per window the progressing reader takes: 8 MiB
/// per window at the default floor.
pub const PROGRESS_FLOORS_PER_WINDOW: usize = 512;

/// The fewest multiples of `ConnLimits::egress_floor` per window the reader must actually have
/// taken, over each of two windows, for a pass to mean progress was made: 1 MiB a window, about one
/// Linux lump. It is below [`PROGRESS_FLOORS_PER_WINDOW`] because the reader takes what the socket
/// has accepted, and on Windows the accepted lumps hold the read under the requested rate.
pub const MIN_FLOORS_PER_WINDOW_READ: usize = 64;

/// The longest gap between two socket acceptances the progressing reader may see before the
/// control stops meaning anything: half a window of one second. A gap of a whole window with fewer
/// than the floor accepted is what the connection unit cuts, so a gap this long is the premise
/// failing, not the unit.
pub const LUMP_GAP_BOUND: Duration = Duration::from_millis(500);

/// The bytes per window a progressing reader takes: [`PROGRESS_FLOORS_PER_WINDOW`] floors at the
/// default `ConnLimits::egress_floor`.
pub fn progress_bytes_per_window() -> usize {
    PROGRESS_FLOORS_PER_WINDOW * ConnLimits::default().egress_floor
}

/// When, and by how much, the server's socket accepted more bytes while a progressing reader read.
///
/// The grants are the figure the connection unit judges, so a cut that follows a gap in them is the
/// unit reading the socket's lumps, not the reader stalling.
#[derive(Debug)]
pub struct Acceptances {
    /// The instant the reader began; every grant's time is measured from it.
    start: Instant,
    /// Each sample at which the socket had accepted more: the time since `start` and the bytes
    /// accepted since the previous grant.
    grants: Vec<(Duration, u64)>,
    /// The accepted total at the previous grant.
    last: u64,
}

impl Acceptances {
    /// A record whose clock starts now.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            grants: Vec::new(),
            last: 0,
        }
    }

    /// Records the socket's accepted total (`WriteStats::accepted`) as seen now.
    pub fn sample(&mut self, accepted: u64) {
        self.sample_at(self.start.elapsed(), accepted);
    }

    /// Records the socket's accepted total as seen `at` after the start. A total equal to the
    /// previous one adds no grant.
    pub fn sample_at(&mut self, at: Duration, accepted: u64) {
        if accepted != self.last {
            self.grants.push((at, accepted - self.last));
            self.last = accepted;
        }
    }

    /// The widest gap between two consecutive grants. The first grant is the buffer filling before
    /// the reader paces it, so the gaps start at the second.
    pub fn widest_gap(&self) -> Duration {
        self.grants
            .windows(2)
            .skip(1)
            .map(|pair| pair[1].0 - pair[0].0)
            .max()
            .unwrap_or_default()
    }

    /// What the reader saw, for a failure message: the bytes it had read and every grant as
    /// (milliseconds since the start, bytes).
    pub fn evidence(&self, read: usize) -> String {
        let grants: Vec<(u128, u64)> = self
            .grants
            .iter()
            .map(|(at, bytes)| (at.as_millis(), *bytes))
            .collect();
        format!("read {read} B; socket accepted (ms, B) {grants:?}")
    }

    /// Asserts the premise a keeps-reading control rests on: the reader took at least two windows
    /// of [`MIN_FLOORS_PER_WINDOW_READ`] floors, and no gap between acceptances reached
    /// [`LUMP_GAP_BOUND`].
    ///
    /// # Panics
    ///
    /// Panics, naming the figure and carrying [`Acceptances::evidence`], when either bound fails.
    pub fn assert_premise(&self, read: usize) {
        let least = 2 * MIN_FLOORS_PER_WINDOW_READ * ConnLimits::default().egress_floor;
        assert!(
            read >= least,
            "the reader took {read} B, under the {least} B the control needs to mean anything"
        );
        let widest = self.widest_gap();
        assert!(
            widest < LUMP_GAP_BOUND,
            "the socket went {} ms between acceptances, too long for the control to mean anything: {}",
            widest.as_millis(),
            self.evidence(read)
        );
    }
}

impl Default for Acceptances {
    fn default() -> Self {
        Self::new()
    }
}
