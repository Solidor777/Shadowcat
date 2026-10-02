//! Where the HTTP/2 stream unit starts cutting a paced reader. A pending chunk waits for send
//! credit; a reader that returns credit slower than the chunk's wait allows is cut once the wait
//! reaches `ConnLimits::egress_window`.
//!
//! The grid's `hypothesis_Bps` column is `min(W / 2, EGRESS_FLOOR_BYTES) / egress_window`, the rate
//! at which one 16 KiB chunk's wait equals the window. The measurements follow it at the extremes
//! only: a reader well below it is always cut and one far above it never is, but a band around and
//! above it (up to about twice the column's value) is cut in some runs and not in others. The
//! numbers are measured with the `h2` crate client over loopback, so a browser's own window and
//! credit batching are not covered.
//!
//! The blocking test runs at a pinned window and two rates, each inside a zone the measurements
//! found always cut or never cut. `boundary_grid` and `boundary_mixed_zone_serial` are measurements,
//! not assertions: they print one `RESULT` line per configuration and are run by hand.

use std::time::Duration;

use h2::Reason;
use shadowcat::http::conn_serve::ConnLimits;
use shadowcat_test_support::egress::{get_path, paced_read, EgressServer, Paced, PacedEnd};

/// A server with the given no-progress window and a short drain.
async fn serve(window: Duration) -> EgressServer {
    EgressServer::start(ConnLimits {
        egress_window: window,
        drain_timeout: Duration::from_millis(500),
        ..ConnLimits::default()
    })
    .await
}

/// What one paced read saw, from the client and from the server's drain report.
struct Run {
    /// The client's view: how its read ended.
    paced: Paced,
    /// Streams the server cut over the run, from the drain report. The server's count is the
    /// authority: a client yields the DATA it has already buffered before it reports a reset, so
    /// behind a large window it sees the reset only after reading everything the server sent.
    cuts: usize,
}

/// Reads `/endless` at `rate` bytes per second behind a stream window of `stream_window` bytes
/// (the connection window is 32 MiB so it never limits), for at most `limit`.
async fn read_at(window: Duration, stream_window: u32, rate: u64, limit: Duration) -> Run {
    let server = serve(window).await;
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

/// The rate at which the hypothesis puts the boundary, bytes per second.
fn hypothesis(window: Duration, stream_window: u32) -> f64 {
    let chunk = (stream_window as f64 / 2.0).min(16.0 * 1024.0);
    chunk / window.as_secs_f64()
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "measurement grid, about a minute: cargo test --test egress_boundary -- --ignored --nocapture"]
async fn boundary_grid() {
    let one = Duration::from_secs(1);
    let mut runs = Vec::new();
    // Large windows at the rates the hypothesis puts above its threshold, and one below as the
    // control: 1 s window.
    for stream_window in [1u32 << 20, 6 << 20] {
        for rate in [
            8_192u64, 16_384, 20_480, 24_576, 28_672, 32_768, 40_960, 49_152,
        ] {
            runs.push((one, stream_window, rate, Duration::from_secs(25)));
        }
    }
    // The pinned window of the blocking test and its neighbours, at 1 s, for a shorter run.
    for stream_window in [32u32 * 1024, 64 * 1024, 256 * 1024] {
        for rate in [
            4_096u64, 8_192, 12_288, 16_384, 18_432, 24_576, 32_768, 40_960, 49_152, 65_536,
        ] {
            runs.push((one, stream_window, rate, Duration::from_secs(15)));
        }
    }
    // One window above 1 s, at rates on each side of the hypothesis for it.
    let three = Duration::from_secs(3);
    for stream_window in [64u32 * 1024, 1 << 20] {
        let boundary = hypothesis(three, stream_window);
        for mult in [0.5f64, 0.75, 1.5, 2.0] {
            runs.push((
                three,
                stream_window,
                (boundary * mult) as u64,
                Duration::from_secs(40),
            ));
        }
    }
    let results = futures_util::future::join_all(runs.iter().map(
        |&(window, stream_window, rate, limit)| async move {
            read_at(window, stream_window, rate, limit).await
        },
    ))
    .await;
    for ((window, stream_window, rate, limit), run) in runs.iter().zip(results) {
        println!(
            "RESULT egress_window={}s W={} reader_Bps={} hypothesis_Bps={:.0} limit={}s server_cuts={} client_end={:?} elapsed_s={:.1} bytes_read={}",
            window.as_secs(),
            stream_window,
            rate,
            hypothesis(*window, *stream_window),
            limit.as_secs(),
            run.cuts,
            run.paced.end,
            run.paced.elapsed.as_secs_f64(),
            run.paced.got
        );
    }
}

/// The rates between "always cut" and "never cut" in `boundary_grid`, one server at a time and
/// repeated, to tell load from the grid's twelve concurrent servers apart from behaviour of the
/// unit itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "measurement, about three minutes: cargo test --test egress_boundary -- --ignored --nocapture boundary_mixed_zone_serial"]
async fn boundary_mixed_zone_serial() {
    let one = Duration::from_secs(1);
    for repeat in 0..2 {
        for rate in [24_576u64, 28_672, 32_768] {
            let run = read_at(one, 1 << 20, rate, Duration::from_secs(25)).await;
            println!(
                "RESULT serial repeat={repeat} egress_window=1s W=1048576 reader_Bps={rate} server_cuts={} client_end={:?} elapsed_s={:.1} bytes_read={}",
                run.cuts, run.paced.end, run.paced.elapsed.as_secs_f64(), run.paced.got
            );
        }
    }
}
