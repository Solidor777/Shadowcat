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
//! - `h1-progress <windows> <multiples> [windows_per_run [recv_buffer_bytes]]`: an HTTP/1 reader that takes
//!   `multiple x ConnLimits::egress_floor` bytes per window, for each comma-separated window (in
//!   seconds) and multiple, one run at a time. Each run prints when, and how many bytes at a
//!   time, the server's socket accepted writes (the figure the connection unit's sampler reads),
//!   and whether and when the connection was cut. `recv_buffer_bytes` sets the reader socket's
//!   receive buffer. Example: `h1-progress 1,3,10 1.5,2,4`.
//!
//! - `layers [downloads [runs [workers]]]`: loopback download throughput of a ladder of servers over a bare
//!   hyper server (the router, the application's middleware, the serve loop, the harness's
//!   observed source), with a body-producer control and TCP_NODELAY off as controls. The ladder is
//!   a tree: each rung names the rung it is compared with, and the three `-no-nodelay` rungs
//!   repeat a nodelay rung with the option off. Run it in a release build. Prints one
//!   `RECORD layers` line per run and per rung and a `RESULT layers` summary of the median over
//!   runs, where every rung but the root also prints its `baseline` and
//!   `throughput_vs_baseline` (the baseline's median over the rung's). `workers` is the runtime's
//!   worker thread count (default `DEFAULT_WORKERS`).
//!
//! The grid's `hypothesis_Bps` column is `measure::hypothesis`. Readers are the `h2` crate's
//! client over loopback, so a browser's own window and credit batching are not covered.

use std::sync::atomic::Ordering;
use std::time::Duration;

use shadowcat::http::conn_serve::ConnLimits;
use shadowcat_test_support::egress::{EgressServer, Protocol};
use shadowcat_test_support::measure::{hypothesis, read_at, record, Run};
use shadowcat_test_support::throughput::{parse_worker_count, DEFAULT_WORKERS};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

/// Runs the subcommand named by the first argument.
fn main() {
    let mut args = std::env::args().skip(1);
    let sub = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let workers = if sub == "grid" {
        8
    } else if sub == "layers" {
        parse_worker_count(rest.get(2).map(String::as_str)).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        })
    } else {
        DEFAULT_WORKERS
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("runtime");
    match sub.as_str() {
        "grid" => runtime.block_on(grid()),
        "mixed-serial" => runtime.block_on(mixed_serial()),
        "h1-progress" => {
            let windows = parse_list(rest.first().map(String::as_str).unwrap_or("1,3,10"));
            let multiples = parse_list(rest.get(1).map(String::as_str).unwrap_or("1.5,2,4"));
            let per_run = rest.get(2).and_then(|v| v.parse().ok()).unwrap_or(3.0f64);
            let recv_buffer = rest.get(3).and_then(|v| v.parse().ok()).unwrap_or(0u32);
            runtime.block_on(h1_progress(&windows, &multiples, per_run, recv_buffer));
        }
        "layers" => {
            let downloads = rest
                .first()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200usize);
            let runs = rest.get(1).and_then(|v| v.parse().ok()).unwrap_or(5usize);
            runtime.block_on(layers(downloads, runs));
        }
        "socket-grants" => {
            let nums: Vec<u32> = rest.iter().filter_map(|v| v.parse().ok()).collect();
            let at = |i: usize| nums.get(i).copied().unwrap_or(0);
            runtime.block_on(socket_grants(
                at(0),
                at(1),
                u64::from(at(2).max(1)),
                (at(3) as usize).max(1),
            ));
        }
        other => {
            eprintln!(
                "unknown subcommand {other:?}: grid | mixed-serial | h1-progress | socket-grants | layers"
            );
            std::process::exit(2);
        }
    }
}

/// The HTTP/1.1 request the probe sends.
const REQUEST: &[u8] = b"GET /endless HTTP/1.1\r\nHost: test\r\n\r\n";

/// Parses a comma-separated list of numbers, exiting on a malformed entry.
fn parse_list(text: &str) -> Vec<f64> {
    text.split(',')
        .map(|v| {
            v.trim().parse().unwrap_or_else(|_| {
                eprintln!("not a number: {v:?}");
                std::process::exit(2)
            })
        })
        .collect()
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

/// The `layers` subcommand: `downloads` per rung per run, `runs` runs, the rungs' order rotating
/// within a run so no rung always follows the same neighbour.
async fn layers(downloads: usize, runs: usize) {
    use shadowcat_test_support::throughput::{
        bare_server, baseline_index, bulk_path, download, full_app, median, minimal_app, served,
        BareApp, Rung, PRODUCTION_SEND_BUF,
    };
    let workers = tokio::runtime::Handle::current().metrics().num_workers();
    let path = bulk_path();
    let default_buf = None;
    let sixty_four = Some(PRODUCTION_SEND_BUF);
    // Every rung but the three marked "no-nodelay" sets TCP_NODELAY, as every production accept
    // path does. Each rung's `over` names the rung its step is read against, so a step is one
    // layer (or, for a no-nodelay rung, the option turned off) and never a comparison across
    // several.
    let rungs: Vec<Rung> = vec![
        Rung::bare(
            "bare-default-buf",
            bare_server(default_buf, true, BareApp::Chunks).await,
        ),
        Rung::bare(
            "bare-64k",
            bare_server(sixty_four, true, BareApp::Chunks).await,
        )
        .over("bare-default-buf"),
        Rung::bare(
            "bare-64k-onebuffer-producer",
            bare_server(sixty_four, true, BareApp::one_buffer()).await,
        )
        .over("bare-64k"),
        Rung::bare(
            "bare-64k+minimal-router",
            bare_server(sixty_four, true, BareApp::Router(minimal_app())).await,
        )
        .over("bare-64k"),
        Rung::bare(
            "bare-64k+full-router",
            bare_server(sixty_four, true, BareApp::Router(full_app().await)).await,
        )
        .over("bare-64k+minimal-router"),
        Rung::served(
            "serve_router+minimal-router",
            served(minimal_app(), true).await,
        )
        .over("bare-64k+minimal-router"),
        Rung::served(
            "serve_router+full-router",
            served(full_app().await, true).await,
        )
        .over("bare-64k+full-router"),
        Rung::harness(
            "serve_router+full-router+observed-source",
            EgressServer::start_tuned(ConnLimits::default(), Protocol::H2, true).await,
        )
        .over("serve_router+full-router"),
        Rung::bare(
            "bare-64k-no-nodelay",
            bare_server(sixty_four, false, BareApp::Chunks).await,
        )
        .over("bare-64k"),
        Rung::served(
            "serve_router+full-router-no-nodelay",
            served(full_app().await, false).await,
        )
        .over("serve_router+full-router"),
        Rung::harness(
            "serve_router+full-router+observed-source-no-nodelay",
            EgressServer::start_tuned(ConnLimits::default(), Protocol::H2, false).await,
        )
        .over("serve_router+full-router+observed-source"),
    ];
    for rung in &rungs {
        download(rung.addr, &path).await;
    }
    let mut medians: Vec<Vec<Duration>> = vec![Vec::new(); rungs.len()];
    for run in 1..=runs {
        let mut samples: Vec<Vec<Duration>> = vec![Vec::new(); rungs.len()];
        for i in 0..downloads {
            let mut order: Vec<usize> = (0..rungs.len()).collect();
            order.rotate_left((i + run) % rungs.len());
            if i % 2 == 1 {
                order.reverse();
            }
            for rung in order {
                samples[rung].push(download(rungs[rung].addr, &path).await);
            }
        }
        for (rung, s) in samples.iter_mut().enumerate() {
            let m = median(s);
            record(&format!(
                "RECORD layers os={} profile={} workers={workers} run={run} downloads={downloads} rung=\"{}\" median_ms={:.2}",
                std::env::consts::OS,
                if cfg!(debug_assertions) { "debug" } else { "release" },
                rungs[rung].name,
                m.as_secs_f64() * 1e3
            ));
            medians[rung].push(m);
        }
    }
    let bare = median(&mut medians[0].clone()).as_secs_f64();
    let rung_medians: Vec<f64> = medians
        .iter_mut()
        .map(|m| median(m).as_secs_f64())
        .collect();
    for (rung, m) in medians.iter().enumerate() {
        let min = m.iter().min().copied().unwrap_or_default().as_secs_f64();
        let max = m.iter().max().copied().unwrap_or_default().as_secs_f64();
        let med = rung_medians[rung];
        let step = match baseline_index(&rungs, rung) {
            Some(b) => format!(
                " baseline=\"{}\" throughput_vs_baseline={:.3}",
                rungs[b].name,
                rung_medians[b] / med
            ),
            None => String::new(),
        };
        record(&format!(
            "RESULT layers rung=\"{}\" median_of_run_medians_ms={:.2} run_min_ms={:.2} run_max_ms={:.2} throughput_vs_bare_default={:.3}{step}",
            rungs[rung].name,
            med * 1e3,
            min * 1e3,
            max * 1e3,
            bare / med,
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

/// One HTTP/1 reader per (window, multiple): `multiple` floors per window, for `windows_per_run`
/// windows or until the server drops the connection. `recv_buffer` sets the reader socket's
/// receive buffer in bytes (0 leaves the system default).
async fn h1_progress(windows: &[f64], multiples: &[f64], windows_per_run: f64, recv_buffer: u32) {
    let floor = ConnLimits::default().egress_floor;
    for &window_secs in windows {
        for &multiple in multiples {
            let window = Duration::from_secs_f64(window_secs);
            let server = EgressServer::start_with(
                ConnLimits {
                    egress_window: window,
                    drain_timeout: Duration::from_millis(500),
                    ..ConnLimits::default()
                },
                Protocol::H1,
            )
            .await;
            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            if recv_buffer > 0 {
                socket.set_recv_buffer_size(recv_buffer).unwrap();
            }
            let mut peer = socket.connect(server.addr).await.unwrap();
            peer.write_all(REQUEST).await.unwrap();
            let per_window = multiple * floor as f64;
            let step = Duration::from_millis(100);
            let chunk = ((per_window * step.as_secs_f64() / window_secs) as usize).max(1);
            let mut buf = vec![0u8; chunk];
            let run_for = window.mul_f64(windows_per_run);
            let start = Instant::now();
            let mut last = 0u64;
            // (milliseconds since the request, bytes the socket accepted since the last sample),
            // for the samples that accepted anything: the writer's view of the reader's progress.
            let mut grants: Vec<(u128, u64)> = Vec::new();
            let mut read = 0usize;
            let mut cut_at = None;
            while start.elapsed() < run_for {
                tokio::time::sleep(step).await;
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_millis(200), peer.read(&mut buf)).await
                {
                    read += n;
                }
                let accepted = server.writes.accepted.load(Ordering::Relaxed);
                if accepted != last {
                    grants.push((start.elapsed().as_millis(), accepted - last));
                    last = accepted;
                }
                if let Some(at) = server.closed.lock().unwrap().first().copied() {
                    cut_at = Some(at.duration_since(start));
                    break;
                }
            }
            let parks = server.writes.parks.load(Ordering::Relaxed);
            drop(peer);
            let drain = server.finish().await;
            record(&format!(
                "RESULT h1 egress_window={window_secs}s multiple={multiple} recv_buffer={recv_buffer} floor_B={floor} per_window_B={per_window:.0} read_B={read} cut_at={cut_at:?} write_parks={parks} stall_cuts={:?}",
                drain.stall_cuts
            ));
            record(&format!("RESULT h1   accepted_grants (ms, B)={grants:?}"));
        }
    }
}

/// A bare loopback socket pair with no server in the way: the writer fills the send path until a
/// write would block, then waits for writability and writes again; the reader takes
/// `rate` bytes per second. Prints when, and how many bytes at a time, the writer was let back in:
/// the operating system's grant granularity for a slow reader. `send_buffer` and `recv_buffer`
/// (bytes; 0 leaves the default) set the listener's and the reader's buffers; `block_len` is the
/// size of each write offered.
async fn socket_grants(send_buffer: u32, recv_buffer: u32, rate: u64, block_len: usize) {
    let listener_socket = tokio::net::TcpSocket::new_v4().unwrap();
    if send_buffer > 0 {
        listener_socket.set_send_buffer_size(send_buffer).unwrap();
    }
    listener_socket
        .bind("127.0.0.1:0".parse().unwrap())
        .unwrap();
    let listener = listener_socket.listen(16).unwrap();
    let addr = listener.local_addr().unwrap();
    let reader_socket = tokio::net::TcpSocket::new_v4().unwrap();
    if recv_buffer > 0 {
        reader_socket.set_recv_buffer_size(recv_buffer).unwrap();
    }
    let (reader, accepted) = tokio::join!(reader_socket.connect(addr), listener.accept());
    let mut reader = reader.unwrap();
    let (writer, _) = accepted.unwrap();
    let start = Instant::now();
    let writer_task = tokio::spawn(async move {
        let block = vec![0u8; block_len];
        let mut grants: Vec<(u128, usize)> = Vec::new();
        let mut run = 0usize;
        while start.elapsed() < Duration::from_secs(8) {
            match writer.try_write(&block) {
                Ok(n) => run += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if run > 0 {
                        grants.push((start.elapsed().as_millis(), run));
                        run = 0;
                    }
                    if tokio::time::timeout(Duration::from_secs(1), writer.writable())
                        .await
                        .is_err()
                    {
                        continue;
                    }
                }
                Err(_) => break,
            }
        }
        grants
    });
    let step = Duration::from_millis(100);
    let mut buf = vec![0u8; ((rate as f64 * step.as_secs_f64()) as usize).max(1)];
    let mut read = 0usize;
    while start.elapsed() < Duration::from_secs(8) {
        tokio::time::sleep(step).await;
        if let Ok(Ok(n)) =
            tokio::time::timeout(Duration::from_millis(200), reader.read(&mut buf)).await
        {
            read += n;
        }
    }
    let grants = writer_task.await.unwrap();
    record(&format!(
        "RESULT socket send_buffer={send_buffer} recv_buffer={recv_buffer} reader_Bps={rate} block_B={block_len} read_B={read} write_grants (ms, B)={grants:?}"
    ));
}
