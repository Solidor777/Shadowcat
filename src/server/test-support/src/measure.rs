//! This module supports measurement: paced reads against a server with a given no-progress window,
//! and the record sink for values a test measures but does not yet assert on.
//!
//! A measured value that is only empirical (a scheduling overshoot, a throughput ratio) is
//! recorded, not asserted, until every operating system's value has been recorded; the sink
//! writes each record to standard output and, when the CI runner provides one, to its step
//! summary, so a passing test's record is visible per operating-system leg.

use std::io::Write;
use std::time::Duration;

use shadowcat::http::conn_serve::{ConnLimits, EGRESS_CHUNK_BYTES};
use tokio::time::Instant;

use crate::egress::{get_path, paced_read, EgressServer, Paced};
use crate::observed::WriteStats;

/// Writes one record line to standard output and appends it to the file named by the runner's
/// `GITHUB_STEP_SUMMARY` when that is set. A failure to append is reported on standard error and
/// never fails the caller: a record is evidence, not a result.
pub fn record(line: &str) {
    println!("{line}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let appended = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .and_then(|mut file| writeln!(file, "\n{line}\n"));
        if let Err(e) = appended {
            eprintln!("record: could not append to the step summary: {e}");
        }
    }
}

/// The most a cut may land after its derived bound, as a ceiling on `observed - derived` in
/// [`record_cut_overshoot`]. What lies beyond the derived bound is timer and task wake-up latency,
/// which no premise bounds, so the ceiling is empirical: the worst overshoot recorded on each
/// operating-system leg (Windows 570 ms, Linux 288 ms, macOS 9 ms; the cuts that land before their
/// derived bound read negative) with 2000 ms set above it, 3.5 times the worst leg.
pub const CUT_OVERSHOOT_MAX: Duration = Duration::from_millis(2000);

/// Records how long after its derived bound a cut was observed, and asserts the overshoot is at
/// most [`CUT_OVERSHOOT_MAX`].
///
/// `derived` is the part of the delay the design fixes for the path the caller measures (for a
/// sampler cut, the window plus one sampler tick: the cut lands at the first tick at or after the
/// window; for a shutdown, the bound at which the session's IO is killed or its send is cut). What
/// lies beyond it is timer and task wake-up latency, which no premise bounds; the record carries
/// it per operating system. The ceiling catches a cut observed more than `CUT_OVERSHOOT_MAX`
/// (2 s) past `derived`: on a one-second window that is a cut more than two whole windows late, so a
/// cut one extra window late passes.
///
/// # Panics
///
/// Panics under [`assert_cut_overshoot_within_ceiling`]; the record is written first, so the
/// failing value is in the log.
pub fn record_cut_overshoot(what: &str, observed: Duration, derived: Duration) {
    let overshoot_ms = observed.as_secs_f64() * 1e3 - derived.as_secs_f64() * 1e3;
    record(&format!(
        "RECORD cut_overshoot os={} what=\"{what}\" observed_ms={:.0} derived_ms={:.0} overshoot_ms={overshoot_ms:.0}",
        std::env::consts::OS,
        observed.as_secs_f64() * 1e3,
        derived.as_secs_f64() * 1e3,
    ));
    assert_cut_overshoot_within_ceiling(what, observed, derived);
}

/// Records how long after the connection unit's derived bound a cut landed, measured from the
/// last instant the server's socket accepted `floor` bytes of progress.
///
/// The unit's window runs from that last progress, so it is the reference the derived bound
/// (`window` plus one `tick`) holds from. The producer going quiet is not: a socket whose peer has
/// stopped reading can still accept a later lump after the producer has been flat for a while,
/// and the window restarts at each. The overshoot is recorded and capped like
/// [`record_cut_overshoot`].
///
/// # Panics
///
/// Panics when the socket never accepted `floor` bytes, or under
/// [`assert_cut_overshoot_within_ceiling`].
pub fn record_cut_after_progress(
    what: &str,
    closed_at: Instant,
    writes: &WriteStats,
    floor: usize,
    window: Duration,
    tick: Duration,
) {
    let progress_at = writes
        .last_progress_at(floor as u64)
        .unwrap_or_else(|| panic!("{what}: the socket never accepted {floor} B"));
    record_cut_overshoot(
        what,
        closed_at.saturating_duration_since(progress_at),
        window + tick,
    );
}

/// Asserts `observed` is at most `derived + CUT_OVERSHOOT_MAX`. Separate from
/// [`record_cut_overshoot`] so the ceiling is testable without writing a record.
///
/// # Panics
///
/// Panics, naming `what`, when `observed` exceeds `derived + CUT_OVERSHOOT_MAX`.
pub fn assert_cut_overshoot_within_ceiling(what: &str, observed: Duration, derived: Duration) {
    assert!(
        observed <= derived + CUT_OVERSHOOT_MAX,
        "{what}: cut at {observed:?}, more than {CUT_OVERSHOOT_MAX:?} past the derived {derived:?}"
    );
}

/// What one paced read saw, from the client and from the server's drain report.
pub struct Run {
    /// The client's view: how its read ended.
    pub paced: Paced,
    /// Streams the server cut over the run, from the drain report. The server's count is the
    /// authority: a client yields the DATA it has already buffered before it reports a reset, so
    /// behind a large window it sees the reset only after reading everything the server sent.
    pub cuts: usize,
}

/// Reads `/endless` over HTTP/2 at `rate` bytes per second behind a stream window of
/// `stream_window` bytes (the connection window is 32 MiB so it never limits), against a server
/// whose no-progress window is `window`, for at most `limit`.
pub async fn read_at(window: Duration, stream_window: u32, rate: u64, limit: Duration) -> Run {
    let server = EgressServer::start(ConnLimits {
        egress_window: window,
        drain_timeout: Duration::from_millis(500),
        ..ConnLimits::default()
    })
    .await;
    let mut client = server.connect(stream_window, 32 << 20).await;
    let mut body = get_path(&mut client, server.addr, "/endless")
        .await
        .into_body();
    let paced = paced_read(&mut body, rate, limit).await;
    drop(client);
    let drain = server.finish().await;
    Run {
        paced,
        cuts: drain.stall_cuts.stream,
    }
}

/// The rate at which one response chunk's wait for credit equals `window`, bytes per second:
/// `min(stream_window / 2, EGRESS_CHUNK_BYTES) / window`. The stream unit's boundary is the chunk
/// the body is sliced into, so the chunk constant is its one source here;
/// `ConnLimits::egress_floor` does not enter.
pub fn hypothesis(window: Duration, stream_window: u32) -> f64 {
    let chunk = (stream_window as f64 / 2.0).min(EGRESS_CHUNK_BYTES as f64);
    chunk / window.as_secs_f64()
}
