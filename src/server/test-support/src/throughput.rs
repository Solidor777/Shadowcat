//! Loopback download throughput of an HTTP/2 server, and the ladder of servers that separates the
//! production path's layers from a bare hyper server.
//!
//! One download is a fresh connection fetching [`BODY_BYTES`] with a 6 MiB stream window and a
//! 15 MiB connection window, so the client never limits the server. Every variant answers
//! `/bulk?bytes=N` with the same bytes, so a difference between two variants is the layer the
//! variant adds.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::Query;
use axum::routing::get;
use axum::Router;
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

use crate::egress::{bulk_body, get_path, EgressServer};
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

/// A server in the ladder: its name, the layer it adds over the variant it is compared with, and
/// its address. Holds what keeps the server alive.
pub struct Rung {
    /// The variant's name.
    pub name: &'static str,
    /// The address downloads go to.
    pub addr: SocketAddr,
    /// Keeps a production serve loop or harness server alive.
    _keep: Option<Box<dyn std::any::Any + Send>>,
}

impl Rung {
    /// A rung over a bare server or any address that needs no handle.
    pub fn bare(name: &'static str, addr: SocketAddr) -> Self {
        Self {
            name,
            addr,
            _keep: None,
        }
    }

    /// A rung over a production serve loop.
    pub fn served(name: &'static str, server: Served) -> Self {
        Self {
            name,
            addr: server.addr,
            _keep: Some(Box::new(server)),
        }
    }

    /// A rung over the egress harness's server (the production router behind its observed source).
    pub fn harness(name: &'static str, server: EgressServer) -> Self {
        Self {
            name,
            addr: server.addr,
            _keep: Some(Box::new(server)),
        }
    }
}

/// The send buffer the production serve loop sets, which every bare rung but the first uses.
pub const PRODUCTION_SEND_BUF: usize = H2_SEND_BUF_BYTES;
