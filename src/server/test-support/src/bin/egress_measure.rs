//! Measurement program for the egress no-progress bounds. These are measurements, not tests: each
//! prints `RESULT` lines and asserts nothing, and runs only when invoked.
//!
//! ```text
//! cargo run -p shadowcat-test-support --bin egress_measure -- <subcommand>
//! ```
//!
//! Subcommands:
//! - `grid`: where the HTTP/2 stream unit starts cutting a paced reader, over stream windows,
//!   reader rates and no-progress windows, many servers at a time (about a minute).
//! - `mixed-serial`: the rates `grid` found cut in some runs and not in others, one server at a
//!   time and repeated, to tell load from the concurrent grid apart from behaviour of the unit
//!   (about three minutes).
//!
//! The grid's `hypothesis_Bps` column is `measure::hypothesis`. Readers are the `h2` crate's
//! client over loopback, so a browser's own window and credit batching are not covered.

use std::time::Duration;

use shadowcat_test_support::measure::{hypothesis, read_at, record, Run};

/// Runs the subcommand named by the first argument.
fn main() {
    let mut args = std::env::args().skip(1);
    let sub = args.next().unwrap_or_default();
    let workers = if sub == "grid" { 8 } else { 2 };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("runtime");
    match sub.as_str() {
        "grid" => runtime.block_on(grid()),
        "mixed-serial" => runtime.block_on(mixed_serial()),
        other => {
            eprintln!("unknown subcommand {other:?}: grid | mixed-serial");
            std::process::exit(2);
        }
    }
}

/// The `grid` subcommand.
async fn grid() {
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
        record(&format!(
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
        ));
    }
}

/// The `mixed-serial` subcommand.
async fn mixed_serial() {
    let one = Duration::from_secs(1);
    for repeat in 0..2 {
        for rate in [24_576u64, 28_672, 32_768] {
            let Run { paced, cuts } = read_at(one, 1 << 20, rate, Duration::from_secs(25)).await;
            record(&format!(
                "RESULT serial repeat={repeat} egress_window=1s W=1048576 reader_Bps={rate} server_cuts={cuts} client_end={:?} elapsed_s={:.1} bytes_read={}",
                paced.end,
                paced.elapsed.as_secs_f64(),
                paced.got
            ));
        }
    }
}
