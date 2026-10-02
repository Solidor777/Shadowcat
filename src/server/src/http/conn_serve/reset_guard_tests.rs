//! The reset limits of [`h2_builder`] against a raw HTTP/2 client. Cancelled requests are never
//! refused for queueing: the connection stops reading frames while [`H2_MAX_PENDING_ACCEPT_RESETS`]
//! reset streams await accept, and resumes as hyper accepts them, so a burst of any size is served
//! through. A connection that keeps resetting streams at a sustained rate far above any page's
//! ends with GOAWAY(ENHANCE_YOUR_CALM).
//!
//! Every burst is one write, so the server's codec holds all of it before hyper accepts any stream:
//! the count of reset-but-unaccepted streams reaches the bound, the condition backpressure acts on.

use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

use super::tests::{h2_frame, H2_OPEN};
use super::*;

/// What the client saw after a burst and one follow-up request.
struct Outcome {
    /// The error code of the GOAWAY that ended the connection, when one arrived.
    goaway: Option<u32>,
    /// Whether the follow-up request, sent after the burst, was answered.
    follow_up_answered: bool,
    /// Whether a PING the burst ends with was acknowledged.
    ping_acked: bool,
}

/// GET https, path "/" by static-table index.
const GET_BLOCK: [u8; 3] = [0x82, 0x87, 0x84];

/// HEADERS carrying a whole request (END_STREAM | END_HEADERS) on `sid`.
fn request(sid: u32) -> Vec<u8> {
    h2_frame(0x1, 0x5, sid, &GET_BLOCK)
}

/// RST_STREAM(CANCEL) on `sid`.
fn cancel(sid: u32) -> Vec<u8> {
    h2_frame(0x3, 0, sid, &[0, 0, 0, 8])
}

/// Serves one connection with the production [`h2_builder`] over a duplex pipe, writes `burst` in
/// one write, then a follow-up request on the next stream id, and reports what came back.
async fn run(burst: Vec<u8>, next_sid: u32) -> Outcome {
    let service = hyper::service::service_fn(|_req: axum::http::Request<Incoming>| async move {
        Ok::<_, io::Error>(axum::http::Response::new(axum::body::Body::from("ok")))
    });
    run_with(burst, next_sid, service).await
}

/// [`run`] against an arbitrary service.
async fn run_with<S>(burst: Vec<u8>, next_sid: u32, service: S) -> Outcome
where
    S: hyper::service::Service<
            axum::http::Request<Incoming>,
            Response = axum::http::Response<axum::body::Body>,
            Error = io::Error,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    let (mut peer, server_io) = tokio::io::duplex(8 << 20);
    let limits = ConnLimits::default();
    let server = tokio::spawn(async move {
        let _ = h2_builder(&limits)
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });
    peer.write_all(H2_OPEN).await.unwrap();
    peer.write_all(&burst).await.unwrap();
    peer.write_all(&request(next_sid)).await.unwrap();
    let mut outcome = Outcome {
        goaway: None,
        follow_up_answered: false,
        ping_acked: false,
    };
    let mut inbound = Vec::new();
    let mut scratch = [0u8; 16 * 1024];
    let _ = timeout(Duration::from_secs(10), async {
        while outcome.goaway.is_none() && !outcome.follow_up_answered && !outcome.ping_acked {
            match peer.read(&mut scratch).await {
                Ok(0) | Err(_) => break,
                Ok(n) => inbound.extend_from_slice(&scratch[..n]),
            }
            while inbound.len() >= 9 {
                let len = u32::from_be_bytes([0, inbound[0], inbound[1], inbound[2]]) as usize;
                if inbound.len() < 9 + len {
                    break;
                }
                let kind = inbound[3];
                let flags = inbound[4];
                let sid =
                    u32::from_be_bytes([inbound[5] & 0x7f, inbound[6], inbound[7], inbound[8]]);
                let payload = inbound[9..9 + len].to_vec();
                inbound.drain(..9 + len);
                if kind == 0x7 && payload.len() >= 8 {
                    outcome.goaway = Some(u32::from_be_bytes([
                        payload[4], payload[5], payload[6], payload[7],
                    ]));
                }
                if kind == 0x6 && flags & 0x1 != 0 {
                    outcome.ping_acked = true;
                }
                if kind == 0x1 && sid == next_sid {
                    outcome.follow_up_answered = true;
                }
            }
        }
    })
    .await;
    server.abort();
    outcome
}

/// `total` cancelled requests in waves of at most `wave`, every wave the way a page cancelling its
/// in-flight requests sends them: all the HEADERS, then all the RST_STREAMs. Returns the bytes and
/// the next free stream id.
fn cancel_waves(total: usize, wave: usize) -> (Vec<u8>, u32) {
    let mut bytes = Vec::new();
    let mut sid = 1u32;
    let mut left = total;
    while left > 0 {
        let first = sid;
        for _ in 0..left.min(wave) {
            bytes.extend(request(sid));
            sid += 2;
        }
        left -= left.min(wave);
        for s in (first..sid).step_by(2) {
            bytes.extend(cancel(s));
        }
    }
    (bytes, sid)
}

/// Resets per burst in the browser-shaped tests: eight times the pending-accept bound, in waves
/// the size of the concurrent-stream limit. Well above the bound and below the sustained-rate
/// limit, which is a multiple of the bound over a window of seconds.
const BROWSER_BURST: usize = 8 * H2_MAX_PENDING_ACCEPT_RESETS;

#[tokio::test]
async fn a_cancel_burst_of_exactly_the_bound_is_served_through() {
    let (burst, next) = cancel_waves(
        H2_MAX_PENDING_ACCEPT_RESETS,
        H2_MAX_CONCURRENT_STREAMS as usize,
    );
    let outcome = run(burst, next).await;
    assert_eq!(
        outcome.goaway, None,
        "a legitimate cancel burst ended the connection"
    );
    assert!(
        outcome.follow_up_answered,
        "the request after the burst was not answered"
    );
}

#[tokio::test]
async fn a_cancel_burst_far_past_the_bound_is_served_through() {
    let (burst, next) = cancel_waves(BROWSER_BURST, H2_MAX_CONCURRENT_STREAMS as usize);
    let outcome = run(burst, next).await;
    assert_eq!(
        outcome.goaway, None,
        "a cancel burst past the pending-accept bound ended the connection"
    );
    assert!(
        outcome.follow_up_answered,
        "the request after the burst was not answered"
    );
}

#[tokio::test]
async fn one_reset_past_the_bound_is_served_through() {
    let mut burst = Vec::new();
    let mut sid = 1u32;
    for _ in 0..=H2_MAX_PENDING_ACCEPT_RESETS {
        burst.extend(request(sid));
        burst.extend(cancel(sid));
        sid += 2;
    }
    let outcome = run(burst, sid).await;
    assert_eq!(outcome.goaway, None);
    assert!(outcome.follow_up_answered);
}

/// Reading resumes without any handler finishing: every handler never answers, and a PING sent
/// after a burst past the bound is still acknowledged. Accepting a pending stream depends on the
/// queue alone, so a connection stopped at the bound always drains.
#[tokio::test]
async fn backpressure_drains_while_every_handler_is_blocked() {
    let service = hyper::service::service_fn(|_req: axum::http::Request<Incoming>| async move {
        std::future::pending::<Result<axum::http::Response<axum::body::Body>, io::Error>>().await
    });
    let (mut burst, next) = cancel_waves(BROWSER_BURST, H2_MAX_CONCURRENT_STREAMS as usize);
    burst.extend(h2_frame(0x6, 0, 0, &[7; 8]));
    let outcome = run_with(burst, next, service).await;
    assert_eq!(outcome.goaway, None);
    assert!(
        outcome.ping_acked,
        "the connection stayed stopped at the bound"
    );
}

#[tokio::test]
async fn a_sustained_reset_flood_ends_the_connection() {
    let (burst, next) = cancel_waves(
        32 * H2_MAX_PENDING_ACCEPT_RESETS,
        H2_MAX_CONCURRENT_STREAMS as usize,
    );
    let outcome = run(burst, next).await;
    assert_eq!(
        outcome.goaway,
        Some(11),
        "a reset flood did not end the connection"
    );
    assert!(!outcome.follow_up_answered);
}
