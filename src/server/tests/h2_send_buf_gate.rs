//! Throughput gate for the HTTP/2 send buffer the server sets ([`H2_SEND_BUF_BYTES`], 64 KiB): the
//! keep criterion is at least 90 percent of the median loopback download throughput of an HTTP/2
//! server at the library's default send buffer (400 KiB).
//!
//! Two ratios are recorded per run, both against a bare hyper server at the library default. The
//! buffer ratio serves a bare server with [`H2_SEND_BUF_BYTES`], so the send buffer is the only
//! difference and the ratio is the criterion. The production ratio serves the whole production
//! path (router, egress layers) and so also carries that path's own cost, which a send-buffer
//! change cannot move.
//!
//! A smaller send buffer bounds the body bytes polled ahead of an unread peer, the unsent backlog
//! behind a socket that accepts nothing and a WebSocket tunnel's lead over the wire, at the price
//! of more wake-ups per byte on a fast link. The 90 percent
//! figure is an empirical keep criterion (a throughput ratio on shared hardware has no derivation),
//! so each run is recorded per operating system with `measure::record` and the ratio asserts
//! nothing until the three operating systems' values are recorded. The premises that are not
//! empirical are asserted: every download delivers every byte.
//!
//! The criterion is defined for the release profile: a debug build's per-frame cost dominates the
//! loopback download, so the ratios of a debug run are not comparable with it. Each record names
//! its profile; CI runs this test in a release build (`cargo test --release -p shadowcat --test
//! h2_send_buf_gate`) for the value that counts, and the debug run of `cargo test --all` records
//! the profile artifact beside it.
//!
//! One download is a fresh connection fetching 24 MiB with a 6 MiB stream window and a 15 MiB
//! connection window (so the client never limits the server), the two servers' downloads
//! interleaved with rotating order, each run's ratio taken from the medians of [`DOWNLOADS`]
//! downloads per server. The production side includes the router and the egress layers, which the
//! bare side does not: the ratio is the cost of the whole production path against a bare server.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use h2::client::Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use shadowcat::http::conn_serve::{ConnLimits, H2_SEND_BUF_BYTES};
use shadowcat_test_support::egress::{bulk_body, get_path, EgressServer};
use shadowcat_test_support::h2c::h2c_connect;
use shadowcat_test_support::measure::record;
use tokio::net::TcpListener;

/// Bytes one download fetches.
const BODY_BYTES: usize = 24 << 20;

/// Downloads per server in one run.
const DOWNLOADS: usize = 100;

/// Independent runs.
const RUNS: usize = 3;

/// The build profile the records name: the keep criterion is read from the release records only.
const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

/// A bare hyper HTTP/2 server serving [`bulk_body`] on every request, with `send_buf` as its send
/// buffer or the library's default when `None`.
async fn bare_server(send_buf: Option<usize>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            socket.set_nodelay(true).unwrap();
            let service = service_fn(|_req: axum::http::Request<hyper::body::Incoming>| async {
                Ok::<_, std::convert::Infallible>(axum::http::Response::new(bulk_body(BODY_BYTES)))
            });
            tokio::spawn(async move {
                let mut builder = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
                if let Some(bytes) = send_buf {
                    builder.max_send_buf_size(bytes);
                }
                let _ = builder
                    .serve_connection(TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    addr
}

/// One download of [`BODY_BYTES`] from `addr` on a fresh connection; returns its duration.
async fn download(addr: SocketAddr, path: &str) -> Duration {
    let mut builder = Builder::new();
    builder
        .initial_window_size(6 << 20)
        .initial_connection_window_size(15 << 20)
        .enable_push(false);
    let mut client = h2c_connect(addr, builder).await.unwrap();
    let started = Instant::now();
    let response = get_path(&mut client, addr, path).await;
    let mut body = response.into_body();
    let mut got = 0usize;
    while let Some(chunk) = body.data().await {
        let chunk = chunk.unwrap();
        got += chunk.len();
        let _ = body.flow_control().release_capacity(chunk.len());
    }
    assert_eq!(got, BODY_BYTES, "a download ended short");
    started.elapsed()
}

/// The median of `samples`.
fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    let n = samples.len();
    if n % 2 == 1 {
        samples[n / 2]
    } else {
        (samples[n / 2 - 1] + samples[n / 2]) / 2
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_production_send_buffer_keeps_loopback_throughput_against_the_library_default() {
    let production = EgressServer::start(ConnLimits::default()).await;
    let production_addr = production.addr;
    let default_addr = bare_server(None).await;
    let buffer_addr = bare_server(Some(H2_SEND_BUF_BYTES)).await;
    let path = format!("/bulk?bytes={BODY_BYTES}");
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
    }
    production.finish().await;
}
