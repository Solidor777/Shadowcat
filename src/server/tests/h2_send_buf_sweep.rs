//! Throughput of candidate HTTP/2 send-buffer sizes against the library default.

use std::time::Duration;

use shadowcat_test_support::measure::record;
use shadowcat_test_support::throughput::{bare_server, download, median, BareApp};

/// Candidate send-buffer sizes in KiB.
const CANDIDATES_KIB: [usize; 6] = [192, 256, 320, 352, 384, 400];

/// Downloads per server in one run.
const DOWNLOADS: usize = 100;

/// Runs per invocation.
const RUNS: usize = 3;

/// Records, per run, each candidate's median download throughput over the default's, and a second
/// default-sized server's (`ctl`), which shares the default's configuration and so reads the
/// measurement noise.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sweep() {
    let default_addr = bare_server(None, true, BareApp::Chunks).await;
    let mut addrs = vec![default_addr, bare_server(None, true, BareApp::Chunks).await];
    for kib in CANDIDATES_KIB {
        addrs.push(bare_server(Some(kib * 1024), true, BareApp::Chunks).await);
    }
    for addr in &addrs {
        download(*addr, "/").await;
    }
    let n = addrs.len();
    for run in 1..=RUNS {
        let mut samples: Vec<Vec<Duration>> = vec![Vec::new(); n];
        for i in 0..DOWNLOADS {
            let mut order: Vec<usize> = (0..n).collect();
            order.rotate_left((i + run) % n);
            if i % 2 == 1 {
                order.reverse();
            }
            for server in order {
                samples[server].push(download(addrs[server], "/").await);
            }
        }
        let default_median = median(&mut samples[0]);
        let control = median(&mut samples[1]);
        let mut line = format!(
            "RECORD send_buf_sweep os={} run={run} default_median_ms={:.2} rctl={:.3}",
            std::env::consts::OS,
            default_median.as_secs_f64() * 1e3,
            default_median.as_secs_f64() / control.as_secs_f64()
        );
        for (k, kib) in CANDIDATES_KIB.iter().enumerate() {
            let m = median(&mut samples[k + 2]);
            line.push_str(&format!(
                " r{kib}={:.3}",
                default_median.as_secs_f64() / m.as_secs_f64()
            ));
        }
        record(&line);
    }
}
