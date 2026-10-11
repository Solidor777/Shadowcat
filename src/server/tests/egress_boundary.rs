//! Where the HTTP/2 stream unit starts cutting a paced reader. A pending chunk waits for send
//! credit; a reader that returns credit slower than the chunk's wait allows is cut once the wait
//! reaches `ConnLimits::egress_window`.
//!
//! The measurement's `hypothesis_Bps` column is `min(W / 2, EGRESS_CHUNK_BYTES) / egress_window`
//! (`measure::hypothesis`), the rate at which one 16 KiB chunk's wait equals the window; it is
//! 16384 B/s at a 1 s window for every stream window `W` from 32 KiB up. The measurements follow it
//! at the extremes only, and where the extremes sit depends on `W`. At a 1 s window and stream
//! windows of 1 MiB and 6 MiB (six runs per cell) a reader at 20480 B/s or below was cut in every
//! run and one at 40960 B/s or above in none; 24576 to 32768 B/s was cut in some runs and not in
//! others. At a 32 KiB stream window (three runs per cell, two from 40960 B/s) every rate up to
//! 24576 B/s was cut and 32768 B/s and above was not. The numbers are measured with the `h2` crate
//! client over loopback, so a browser's own window and credit batching are not covered.
//!
//! The blocking test runs at a pinned window and two rates, each inside a zone the measurements
//! found always cut or never cut. The measurements themselves are not tests: the `egress_measure`
//! program in the test-support crate prints one `RESULT` line per configuration (`grid` and
//! `mixed-serial` subcommands, run by hand).

use std::time::Duration;

use h2::Reason;
use shadowcat::http::conn_serve::EGRESS_CHUNK_BYTES;
use shadowcat_test_support::egress::PacedEnd;
use shadowcat_test_support::measure::{hypothesis, read_at, record};

/// The no-progress window the test runs at.
const WINDOW: Duration = Duration::from_secs(1);

/// The stream window the test runs at.
const STREAM_WINDOW: u32 = 32 * 1024;

/// A reader rate the unit cut in every measured run at stream windows of 32 KiB, 64 KiB, 256 KiB,
/// 1 MiB and 6 MiB, 1 s window: half the hypothesis's threshold (8192 B/s at `STREAM_WINDOW`).
fn always_cut_bps() -> u64 {
    (hypothesis(WINDOW, STREAM_WINDOW) / 2.0) as u64
}

/// A reader rate the unit did not cut in any measured run at stream windows of 32 KiB, 64 KiB and
/// 256 KiB (two runs per cell), 1 s window: four times the hypothesis's threshold (65536 B/s at
/// `STREAM_WINDOW`). The 1 MiB and 6 MiB grids stop at 49152 B/s, so 65536 B/s is not measured
/// there. Between the two rates the outcome at `STREAM_WINDOW` is cut at 18432 and 24576 B/s and
/// not cut at 32768 B/s; at larger stream windows the outcome varies from run to run at 24576 B/s
/// (64 KiB, 256 KiB) and up to 32768 B/s (1 MiB, 6 MiB), which is why neither blocking assertion
/// sits between the rates.
fn never_cut_bps() -> u64 {
    (hypothesis(WINDOW, STREAM_WINDOW) * 4.0) as u64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_far_below_the_boundary_is_cut_and_one_far_above_it_is_not() {
    // The two zones were measured with a 16 KiB chunk (the `egress_measure` program's `grid` subcommand); the
    // rates below follow the chunk by the hypothesis, but where the zones sit relative to it was
    // not measured at another chunk size, so the test names that premise instead of failing on a
    // magnitude when the chunk changes.
    assert_eq!(
        EGRESS_CHUNK_BYTES,
        16 * 1024,
        "the always-cut and never-cut zones were measured at a 16 KiB chunk; remeasure them"
    );
    let never = never_cut_bps();
    let always = always_cut_bps();
    let fast = read_at(WINDOW, STREAM_WINDOW, never, WINDOW * 3).await;
    let slow = read_at(WINDOW, STREAM_WINDOW, always, WINDOW * 12).await;
    // Per operating-system leg: the rates and what each run did, so a leg that moves the zones
    // shows in the step summary even while both assertions hold.
    record(&format!(
        "RECORD egress_boundary os={} window_ms={} stream_window={STREAM_WINDOW} hypothesis_bps={:.0} never_cut_bps={never} cuts_at_never={} always_cut_bps={always} cuts_at_always={}",
        std::env::consts::OS,
        WINDOW.as_millis(),
        hypothesis(WINDOW, STREAM_WINDOW),
        fast.cuts,
        slow.cuts,
    ));
    assert_eq!(
        fast.cuts, 0,
        "a reader at {never} B/s was cut: {:?}",
        fast.paced
    );
    assert_eq!(fast.paced.end, PacedEnd::Deadline);
    assert_eq!(
        slow.cuts, 1,
        "a reader at {always} B/s was not cut: {:?}",
        slow.paced
    );
    assert_eq!(slow.paced.end, PacedEnd::Reset(Reason::CANCEL));
}
