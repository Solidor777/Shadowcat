//! Where the HTTP/2 stream unit starts cutting a paced reader. A pending chunk waits for send
//! credit; a reader that returns credit slower than the chunk's wait allows is cut once the wait
//! reaches `ConnLimits::egress_window`.
//!
//! The measurement's `hypothesis_Bps` column is `min(W / 2, EGRESS_FLOOR_BYTES) / egress_window`, the rate
//! at which one 16 KiB chunk's wait equals the window. The measurements follow it at the extremes
//! only: a reader well below it is always cut and one far above it never is, but a band around and
//! above it (up to about twice the column's value) is cut in some runs and not in others. The
//! numbers are measured with the `h2` crate client over loopback, so a browser's own window and
//! credit batching are not covered.
//!
//! The blocking test runs at a pinned window and two rates, each inside a zone the measurements
//! found always cut or never cut. The measurements themselves are not tests: the `egress_measure`
//! program in the test-support crate prints one `RESULT` line per configuration (`grid` and
//! `mixed-serial` subcommands, run by hand).

use std::time::Duration;

use h2::Reason;
use shadowcat_test_support::egress::PacedEnd;
use shadowcat_test_support::measure::read_at;

/// A reader rate the unit has cut in every measured run at stream windows from 32 KiB up, 1 s
/// window: half the hypothesis's threshold.
const ALWAYS_CUT_BPS: u64 = 8_192;

/// A reader rate the unit has not cut in any measured run at stream windows from 32 KiB up, 1 s
/// window: four times the hypothesis's threshold. Between the two constants the outcome varies
/// from run to run (rates from about 18 KiB/s to 33 KiB/s were cut in some runs and not in
/// others), which is why neither blocking assertion sits there.
const NEVER_CUT_BPS: u64 = 65_536;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_far_below_the_boundary_is_cut_and_one_far_above_it_is_not() {
    let window = Duration::from_secs(1);
    let stream_window = 32 * 1024u32;
    let fast = read_at(window, stream_window, NEVER_CUT_BPS, window * 3).await;
    assert_eq!(
        fast.cuts, 0,
        "a reader at {NEVER_CUT_BPS} B/s was cut: {:?}",
        fast.paced
    );
    assert_eq!(fast.paced.end, PacedEnd::Deadline);
    let slow = read_at(window, stream_window, ALWAYS_CUT_BPS, window * 12).await;
    assert_eq!(
        slow.cuts, 1,
        "a reader at {ALWAYS_CUT_BPS} B/s was not cut: {:?}",
        slow.paced
    );
    assert_eq!(slow.paced.end, PacedEnd::Reset(Reason::CANCEL));
}
