//! Measurement support: paced reads against a server with a given no-progress window, and the
//! record sink for values a test measures but does not yet assert on.
//!
//! A measured value that is only empirical (a scheduling overshoot, a throughput ratio) is
//! recorded, not asserted, until every operating system's value has been recorded; the sink
//! writes each record to standard output and, when the CI runner provides one, to its step
//! summary, so a passing test's record is visible per operating-system leg.

use std::io::Write;
use std::time::Duration;

use shadowcat::http::conn_serve::ConnLimits;

use crate::egress::{get_path, paced_read, EgressServer, Paced};

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

/// Records how long after its derived bound a cut was observed.
///
/// `derived` is the part of the detection delay the design fixes (the window plus one sampler
/// tick: the cut lands at the first tick at or after the window). What lies beyond it is timer
/// and task wake-up latency, which no premise bounds, so the overshoot is recorded per operating
/// system and asserted on nothing. It becomes a blocking bound once the three legs are recorded,
/// set below the worst leg by a stated headroom with the recorded values beside the constant.
pub fn record_cut_overshoot(what: &str, observed: Duration, derived: Duration) {
    let overshoot_ms = observed.as_secs_f64() * 1e3 - derived.as_secs_f64() * 1e3;
    record(&format!(
        "RECORD cut_overshoot os={} what=\"{what}\" observed_ms={:.0} derived_ms={:.0} overshoot_ms={overshoot_ms:.0}",
        std::env::consts::OS,
        observed.as_secs_f64() * 1e3,
        derived.as_secs_f64() * 1e3,
    ));
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

/// The rate at which one `EGRESS_FLOOR_BYTES` chunk's wait for credit equals `window`, bytes per
/// second: `min(stream_window / 2, 16 KiB) / window`.
pub fn hypothesis(window: Duration, stream_window: u32) -> f64 {
    let chunk = (stream_window as f64 / 2.0).min(16.0 * 1024.0);
    chunk / window.as_secs_f64()
}
