//! This module provides loopback download throughput of an HTTP/2 server, and the ladder of servers
//! that separates the production path's layers from a bare hyper server.
//!
//! One download is a fresh connection fetching [`BODY_BYTES`] with a 6 MiB stream window and a
//! 15 MiB connection window, so the client never limits the server. Every variant answers the
//! request `bulk_path` names with the same bytes, so a difference between two variants is the layer
//! the variant adds.

use std::any::Any;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::routing::get;
use axum::Router;
use futures_util::future::BoxFuture;
use h2::client::Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use shadowcat::http::conn_serve::{serve_router, ConnLimits, H2_SEND_BUF_BYTES};
use shadowcat::http::router;
use shadowcat::ws::WsState;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::egress::{bulk_body, get_path, EgressServer, Protocol};
use crate::h2_session::seeded_state;
use crate::h2c::{h2c_connect, H2cSource};

/// Bytes one download fetches.
pub const BODY_BYTES: usize = 24 << 20;

/// What a bare hyper server answers with.
#[derive(Clone)]
pub enum BareApp {
    /// [`bulk_body`]: 16 KiB chunks from a stream.
    Chunks,
    /// One buffer of the whole body: no per-chunk producer work at all. Build it with
    /// [`BareApp::one_buffer`].
    OneBuffer(Bytes),
    /// An axum router served through `TowerToHyperService`.
    Router(Router),
}

impl BareApp {
    /// The single-buffer variant holding [`BODY_BYTES`] of filler. The variant owns the buffer, so
    /// `Chunks` and `Router` servers allocate none.
    pub fn one_buffer() -> Self {
        Self::OneBuffer(Bytes::from(vec![1u8; BODY_BYTES]))
    }
}

/// A bare hyper HTTP/2 server on the loopback with `send_buf` as its send buffer (the library's
/// default when `None`) and `TCP_NODELAY` on accepted sockets when `nodelay`. It uses hyper's
/// `TokioExecutor` and no timer, and none of the production serve loop.
pub async fn bare_server(send_buf: Option<usize>, nodelay: bool, app: BareApp) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            if nodelay {
                socket.set_nodelay(true).unwrap();
            }
            let app = app.clone();
            tokio::spawn(async move {
                let mut builder = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
                if let Some(bytes) = send_buf {
                    builder.max_send_buf_size(bytes);
                }
                let io = TokioIo::new(socket);
                let _ = match app {
                    BareApp::Router(router) => {
                        builder
                            .serve_connection(io, TowerToHyperService::new(router))
                            .await
                    }
                    BareApp::Chunks => {
                        let service =
                            service_fn(|_req: axum::http::Request<hyper::body::Incoming>| async {
                                Ok::<_, std::convert::Infallible>(axum::http::Response::new(
                                    bulk_body(BODY_BYTES),
                                ))
                            });
                        builder.serve_connection(io, service).await
                    }
                    BareApp::OneBuffer(whole) => {
                        let service =
                            service_fn(move |_req: axum::http::Request<hyper::body::Incoming>| {
                                let whole = whole.clone();
                                async move {
                                    Ok::<_, std::convert::Infallible>(axum::http::Response::new(
                                        Body::from(whole),
                                    ))
                                }
                            });
                        builder.serve_connection(io, service).await
                    }
                };
            });
        }
    });
    addr
}

/// An axum router with one route, `/bulk`, answering with [`bulk_body`] of the `bytes` query
/// parameter: the application with none of its middleware.
pub fn minimal_app() -> Router {
    Router::new().route(
        "/bulk",
        get(
            |Query(q): Query<std::collections::HashMap<String, usize>>| async move {
                bulk_body(q.get("bytes").copied().unwrap_or(0))
            },
        ),
    )
}

/// The application's own router with the same `/bulk` route: every layer the application adds.
pub async fn full_app() -> Router {
    let (state, _) = seeded_state().await;
    router(state).await.merge(minimal_app())
}

/// A production serve loop (`serve_router`, with its connection limits at their defaults) over a
/// plain listener whose connections speak HTTP/2, and the handles that end it.
pub struct Served {
    /// The bound address.
    pub addr: SocketAddr,
    /// Resolves the shutdown future.
    shutdown: oneshot::Sender<()>,
    /// The serve loop.
    task: JoinHandle<()>,
}

impl Served {
    /// Ends the loop and waits for it.
    pub async fn finish(self) {
        let _ = self.shutdown.send(());
        let _ = self.task.await;
    }
}

/// Serves `app` through `serve_router` over plain loopback HTTP/2, with `TCP_NODELAY` when
/// `nodelay`.
pub async fn served(app: Router, nodelay: bool) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (source, _end) = H2cSource::new(listener);
    let source = source.with_nodelay(nodelay);
    let ws = WsState::new();
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = serve_router(
            source,
            app,
            async move {
                let _ = shutdown_rx.await;
            },
            ConnLimits::default(),
            move || ws.trigger_shutdown(),
        )
        .await;
        drop(_end);
    });
    Served {
        addr,
        shutdown,
        task,
    }
}

/// One download of [`BODY_BYTES`] from `addr` on a fresh connection; returns its duration.
pub async fn download(addr: SocketAddr, path: &str) -> Duration {
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
pub fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    let n = samples.len();
    if n % 2 == 1 {
        samples[n / 2]
    } else {
        (samples[n / 2 - 1] + samples[n / 2]) / 2
    }
}

/// The path every ladder variant answers.
pub fn bulk_path() -> String {
    format!("/bulk?bytes={BODY_BYTES}")
}

/// A server in the ladder: its name, the name of the rung it is compared with (none for the
/// root), and its address. The ladder is a tree, not a line: each rung names its own baseline, so
/// the difference between the two is the layer the rung adds, or the option it turns off. Holds
/// what keeps the server alive.
pub struct Rung {
    /// The variant's name.
    pub name: &'static str,
    /// The name of the rung this one is compared with; it must precede this rung in the ladder.
    pub baseline: Option<&'static str>,
    /// The address downloads go to.
    pub addr: SocketAddr,
    /// Keeps a production serve loop or harness server alive.
    _keep: Option<Box<dyn std::any::Any + Send>>,
}

impl Rung {
    /// A rung named `name` over `server`, compared with the rung named `baseline` (none for the
    /// root).
    fn from_server(name: &'static str, baseline: Option<&'static str>, server: RungServer) -> Self {
        Self {
            name,
            baseline,
            addr: server.addr,
            _keep: server.keep,
        }
    }
}

/// A started ladder server: the address downloads go to and whatever keeps it alive.
pub struct RungServer {
    /// The address downloads go to.
    addr: SocketAddr,
    /// Keeps a production serve loop or harness server alive.
    keep: Option<Box<dyn Any + Send>>,
}

impl RungServer {
    /// A bare server or any address that needs no handle.
    pub fn bare(addr: SocketAddr) -> Self {
        Self { addr, keep: None }
    }

    /// A production serve loop.
    pub fn served(server: Served) -> Self {
        Self {
            addr: server.addr,
            keep: Some(Box::new(server)),
        }
    }

    /// The egress harness's server (the production router behind its observed source).
    pub fn harness(server: EgressServer) -> Self {
        Self {
            addr: server.addr,
            keep: Some(Box::new(server)),
        }
    }
}

/// A rung that is not started yet: its name, the name of the rung it is compared with, and an
/// async constructor that runs only when the rung is selected, so an unselected rung allocates and
/// binds nothing.
pub struct RungSpec {
    /// The variant's name.
    name: &'static str,
    /// The name of the rung this one is compared with; it must precede this rung in the ladder.
    baseline: Option<&'static str>,
    /// Starts the server.
    build: Box<dyn FnOnce() -> BoxFuture<'static, RungServer> + Send>,
}

impl RungSpec {
    /// A spec whose server is started by `build`.
    pub fn new<F, Fut>(name: &'static str, baseline: Option<&'static str>, build: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = RungServer> + Send + 'static,
    {
        Self {
            name,
            baseline,
            build: Box::new(move || Box::pin(build())),
        }
    }
}

/// The index in `rungs` of the baseline of the rung at `at`, `None` for a root rung. A ladder from
/// [`build_ladder`] has validated every baseline, so the lookup cannot miss.
pub fn baseline_index(rungs: &[Rung], at: usize) -> Option<usize> {
    let name = rungs[at].baseline?;
    rungs.iter().position(|r| r.name == name)
}

/// The runtime worker count the `egress_measure` subcommands other than `grid` use when none is given.
pub const DEFAULT_WORKERS: usize = 2;

/// Parses the optional worker-count argument of `egress_measure layers`: absent means
/// [`DEFAULT_WORKERS`]; a present value must be a positive integer (a runtime with zero workers
/// cannot be built, and a non-number is a typing error that must not fall back to the default).
///
/// ```
/// use shadowcat_test_support::throughput::{parse_worker_count, DEFAULT_WORKERS};
///
/// assert_eq!(parse_worker_count(None), Ok(DEFAULT_WORKERS));
/// assert_eq!(parse_worker_count(Some("4")), Ok(4));
/// assert!(parse_worker_count(Some("0")).is_err());
/// ```
pub fn parse_worker_count(arg: Option<&str>) -> Result<usize, String> {
    match arg {
        None => Ok(DEFAULT_WORKERS),
        Some(text) => match text.parse::<usize>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("worker count {text:?} is not a positive integer")),
        },
    }
}

/// The baseline each entry of `chain` (name, baseline) has once only `names` are selected: its
/// nearest selected ancestor along the baseline chain, or none when no ancestor is selected.
/// Panics before anything is resolved when a declared baseline names no earlier entry (a mistyped
/// or misordered baseline would otherwise print no comparison) or when `names` holds a name no
/// entry has (a mistyped selection would otherwise silently drop a rung).
fn selected_baselines(
    chain: &[(&'static str, Option<&'static str>)],
    names: &[&str],
) -> Vec<Option<&'static str>> {
    for (at, (name, baseline)) in chain.iter().enumerate() {
        let Some(baseline) = baseline else { continue };
        assert!(
            chain.iter().any(|(n, _)| n == baseline),
            "rung {name:?} has no rung named {baseline:?} to compare with"
        );
        assert!(
            chain[..at].iter().any(|(n, _)| n == baseline),
            "the baseline {baseline:?} of rung {name:?} does not precede it in the ladder"
        );
    }
    for name in names {
        assert!(
            chain.iter().any(|(n, _)| n == name),
            "no rung named {name:?} to select"
        );
    }
    chain
        .iter()
        .map(|(_, baseline)| {
            let mut ancestor = *baseline;
            while let Some(a) = ancestor {
                if names.contains(&a) {
                    break;
                }
                ancestor = chain.iter().find(|(n, _)| *n == a).and_then(|(_, b)| *b);
            }
            ancestor
        })
        .collect()
}

/// Starts the rungs of `specs` named in `names` (all of them when `names` is `None`), in ladder
/// order. A selected rung whose baseline is not selected is compared with its nearest selected
/// ancestor along the baseline chain, or with nothing when no ancestor is selected, so a subset
/// still prints the step it can. The baselines and `names` are validated against the ladder before
/// any constructor runs (a mistyped or misordered baseline, or a name no rung has, panics), and a
/// constructor of an unselected rung never runs.
///
/// ```
/// use shadowcat_test_support::throughput::{build_ladder, RungServer, RungSpec};
///
/// # #[tokio::main] async fn main() {
/// let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
/// let specs = vec![
///     RungSpec::new("root", None, move || async move { RungServer::bare(addr) }),
///     RungSpec::new("leaf", Some("root"), move || async move { RungServer::bare(addr) }),
/// ];
/// let rungs = build_ladder(specs, Some(&["leaf"])).await;
/// assert_eq!(rungs.len(), 1);
/// assert_eq!(rungs[0].baseline, None);
/// # }
/// ```
pub async fn build_ladder(specs: Vec<RungSpec>, names: Option<&[&str]>) -> Vec<Rung> {
    let chain: Vec<(&'static str, Option<&'static str>)> =
        specs.iter().map(|s| (s.name, s.baseline)).collect();
    let all: Vec<&str> = chain.iter().map(|(n, _)| *n).collect();
    let resolved = selected_baselines(&chain, names.unwrap_or(&all));
    let mut rungs = Vec::new();
    for (spec, baseline) in specs.into_iter().zip(resolved) {
        if names.is_some_and(|names| !names.contains(&spec.name)) {
            continue;
        }
        let server = (spec.build)().await;
        rungs.push(Rung::from_server(spec.name, baseline, server));
    }
    rungs
}

/// The ladder `egress_measure layers` measures: bare hyper servers, the router, the application's
/// middleware, the production serve loop and the harness's observed source, with a
/// body-producer control and `TCP_NODELAY` off as controls. Each rung names the rung its step is
/// read against, so a step is one layer (or, for a `-no-nodelay` rung, the option turned off) and
/// never a comparison across several. Every rung but the three marked `-no-nodelay` sets
/// `TCP_NODELAY` the way `impl ConnSource for TcpListener` and `SniffListener` do
/// (`set_accepted_nodelay`).
pub fn ladder() -> Vec<RungSpec> {
    vec![
        RungSpec::new("bare-default-buf", None, || async {
            RungServer::bare(bare_server(None, true, BareApp::Chunks).await)
        }),
        RungSpec::new("bare-64k", Some("bare-default-buf"), || async {
            RungServer::bare(bare_server(Some(PRODUCTION_SEND_BUF), true, BareApp::Chunks).await)
        }),
        RungSpec::new("bare-64k-onebuffer-producer", Some("bare-64k"), || async {
            RungServer::bare(
                bare_server(Some(PRODUCTION_SEND_BUF), true, BareApp::one_buffer()).await,
            )
        }),
        RungSpec::new("bare-64k+minimal-router", Some("bare-64k"), || async {
            RungServer::bare(
                bare_server(
                    Some(PRODUCTION_SEND_BUF),
                    true,
                    BareApp::Router(minimal_app()),
                )
                .await,
            )
        }),
        RungSpec::new(
            "bare-64k+full-router",
            Some("bare-64k+minimal-router"),
            || async {
                RungServer::bare(
                    bare_server(
                        Some(PRODUCTION_SEND_BUF),
                        true,
                        BareApp::Router(full_app().await),
                    )
                    .await,
                )
            },
        ),
        RungSpec::new(
            "serve_router+minimal-router",
            Some("bare-64k+minimal-router"),
            || async { RungServer::served(served(minimal_app(), true).await) },
        ),
        RungSpec::new(
            "serve_router+full-router",
            Some("bare-64k+full-router"),
            || async { RungServer::served(served(full_app().await, true).await) },
        ),
        RungSpec::new(
            "serve_router+full-router+observed-source",
            Some("serve_router+full-router"),
            || async {
                RungServer::harness(
                    EgressServer::start_tuned(ConnLimits::default(), Protocol::H2, true).await,
                )
            },
        ),
        RungSpec::new("bare-64k-no-nodelay", Some("bare-64k"), || async {
            RungServer::bare(bare_server(Some(PRODUCTION_SEND_BUF), false, BareApp::Chunks).await)
        }),
        RungSpec::new(
            "serve_router+full-router-no-nodelay",
            Some("serve_router+full-router"),
            || async { RungServer::served(served(full_app().await, false).await) },
        ),
        RungSpec::new(
            "serve_router+full-router+observed-source-no-nodelay",
            Some("serve_router+full-router+observed-source"),
            || async {
                RungServer::harness(
                    EgressServer::start_tuned(ConnLimits::default(), Protocol::H2, false).await,
                )
            },
        ),
    ]
}

#[cfg(test)]
mod tests;

/// The send buffer the production serve loop sets, which every bare rung but the first uses.
pub const PRODUCTION_SEND_BUF: usize = H2_SEND_BUF_BYTES;
