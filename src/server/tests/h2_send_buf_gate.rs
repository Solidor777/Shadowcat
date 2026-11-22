//! Throughput gate for the HTTP/2 send buffer the server sets ([`H2_SEND_BUF_BYTES`], 160 KiB).
//!
//! The keep criterion is that the median over runs of the buffer ratio is at least 0.90: the
//! median loopback download throughput at [`H2_SEND_BUF_BYTES`] is at least 90 percent of an
//! HTTP/2 server's at the library's default send buffer (400 KiB).
//!
//! Two ratios are recorded per run, both against a bare hyper server at the library default. The
//! buffer ratio serves a bare server with [`H2_SEND_BUF_BYTES`], so the send buffer is the only
//! difference and the ratio is the criterion. The production ratio serves the whole production
//! path (router, egress layers) and so also carries that path's own cost, which a send-buffer
//! change cannot move.
//!
//! A smaller send buffer bounds the body bytes polled ahead of an unread peer, the unsent backlog
//! behind a socket that accepts nothing and a WebSocket tunnel's lead over the wire, at the price
//! of more wake-ups per byte on a fast link. The 90 percent figure is an empirical keep criterion
//! (a throughput ratio on shared hardware has no derivation), so each run is recorded per operating
//! system with `measure::record`.
//!
//! The criterion is a median over runs, not a per-run bound: on a macOS CI runner two bare servers
//! at the same (default) send buffer read a ratio between 0.85 and 1.10 against each other, so no
//! send-buffer size keeps every single run at 0.90. The blocking assertion is the guard beside the
//! criterion: every release run's buffer ratio is at least [`BUFFER_RATIO_FLOOR`], which sits
//! below the worst run ever recorded at this setting and so fails only on a regression far outside
//! runner noise. The premises that are not empirical are asserted too: every download delivers
//! every byte.
//!
//! The criterion is defined for the release profile: a debug build's per-frame cost dominates the
//! loopback download, so the ratios of a debug run are not comparable with it. Each record names
//! its profile; CI runs this test in a release build (`cargo test --release -p shadowcat --test
//! h2_send_buf_gate`) for the value that counts, and the debug run of `cargo test --all` records
//! the profile artifact beside it.
//!
//! Cost: a release run fetches 24 MiB from three servers [`DOWNLOADS`] times in each of [`RUNS`]
//! runs, 21.1 GiB at 100 and 3; a debug run fetches 8 and 1, 0.56 GiB. At 100 downloads and 3 runs
//! a debug run took 32.5 s on one Windows machine; at 8 and 1 it takes 0.85 s, and its ratios are
//! not the criterion's, so nothing is lost by the smaller median.
//!
//! One download is a fresh connection fetching 24 MiB with a 6 MiB stream window and a 15 MiB
//! connection window (so the client never limits the server), the two servers' downloads
//! interleaved with rotating order, each run's ratio taken from the medians of [`DOWNLOADS`]
//! downloads per server. The production side includes the router and the egress layers, which the
//! bare side does not: the ratio is the cost of the whole production path against a bare server.

use std::time::Duration;

use shadowcat::http::conn_serve::{ConnLimits, H2_SEND_BUF_BYTES};
use shadowcat_test_support::egress::EgressServer;
use shadowcat_test_support::measure::record;
use shadowcat_test_support::throughput::{bare_server, bulk_path, download, median, BareApp};

/// Downloads per server in one run. The release profile's count is the one the keep criterion is
/// read from (a median over 100 per server); a debug run records the profile artifact only, so it
/// moves a twelfth of that.
const DOWNLOADS: usize = if cfg!(debug_assertions) { 8 } else { 100 };

/// Independent runs. One in a debug build, for the reason [`DOWNLOADS`] gives.
const RUNS: usize = if cfg!(debug_assertions) { 1 } else { 3 };

/// The lowest buffer ratio a release run may read. Every release run asserts it; a debug run's
/// ratios are not comparable (see the module documentation) and are not asserted.
///
/// Recorded buffer ratios at [`H2_SEND_BUF_BYTES`] (160 KiB), 100 downloads per median, release
/// build, three runs per operating system in each of two CI runs. First run: Linux 0.950, 0.963,
/// 0.960; macOS 0.912, 0.921, 0.946; Windows 0.914, 0.917, 0.912. Second run: Linux 0.969, 0.958,
/// 0.962; macOS 0.981, 0.892, 0.942; Windows 0.955, 0.957, 0.956. One macOS run read 0.892, under
/// the 0.90 criterion; the median of each leg's six runs is Linux 0.961, macOS 0.932 and Windows
/// 0.936, all at or above it.
///
/// Separately, 24 runs per operating system of a bare server at 160 KiB against the default read a
/// worst of 0.954 on Linux, 0.926 on Windows and 0.810 on macOS. The floor sits 0.110 below that
/// lowest recorded run (macOS 0.810), the worst value it is measured from, so runner noise of the
/// size those runs saw does not reach it.
const BUFFER_RATIO_FLOOR: f64 = 0.70;

/// The build profile the records name: the keep criterion is read from the release records only.
const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_production_send_buffer_keeps_loopback_throughput_against_the_library_default() {
    let production = EgressServer::start(ConnLimits::default()).await;
    let production_addr = production.addr;
    // Both bare servers set TCP_NODELAY, as `impl ConnSource for TcpListener` and `SniffListener`
    // do on the sockets they accept and the harness's source does on its.
    let default_addr = bare_server(None, true, BareApp::Chunks).await;
    let buffer_addr = bare_server(Some(H2_SEND_BUF_BYTES), true, BareApp::Chunks).await;
    let path = bulk_path();
    // Warm every path: first-use costs (allocator, connection setup) belong to no median.
    download(production_addr, &path).await;
    download(default_addr, "/").await;
    download(buffer_addr, "/").await;
    for run in 1..=RUNS {
        let mut samples: [Vec<Duration>; 3] = Default::default();
        for i in 0..DOWNLOADS {
            let mut order = [0usize, 1, 2];
            order.rotate_left((i + run) % 3);
            if i % 2 == 1 {
                order.reverse();
            }
            for server in order {
                let took = match server {
                    0 => download(default_addr, "/").await,
                    1 => download(buffer_addr, "/").await,
                    _ => download(production_addr, &path).await,
                };
                samples[server].push(took);
            }
        }
        let [default_median, buffer_median, production_median] =
            samples.each_mut().map(|s| median(s));
        // Throughput ratio = default-buffer time over the measured server's time.
        let buffer_ratio = default_median.as_secs_f64() / buffer_median.as_secs_f64();
        let production_ratio = default_median.as_secs_f64() / production_median.as_secs_f64();
        record(&format!(
            "RECORD send_buf_gate os={} profile={PROFILE} run={run} downloads={DOWNLOADS} default_median_ms={:.2} buffer_median_ms={:.2} production_median_ms={:.2} buffer_ratio={buffer_ratio:.3} production_ratio={production_ratio:.3}",
            std::env::consts::OS,
            default_median.as_secs_f64() * 1e3,
            buffer_median.as_secs_f64() * 1e3,
            production_median.as_secs_f64() * 1e3,
        ));
        assert!(
            cfg!(debug_assertions) || buffer_ratio >= BUFFER_RATIO_FLOOR,
            "run {run}: the buffer ratio {buffer_ratio:.3} is under the floor {BUFFER_RATIO_FLOOR}"
        );
    }
    production.finish().await;
}
