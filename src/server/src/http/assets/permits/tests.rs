//! These tests cover the permit pool: a bounded wait that ends in a 503 with `Retry-After`,
//! release on drop, and a peak gauge that never goes down.

use super::*;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

#[tokio::test(start_paused = true)]
async fn an_exhausted_pool_refuses_after_exactly_its_wait() {
    let permits = StreamedFilePermits::new(1, Duration::from_millis(700));
    let _held = permits.acquire().await.unwrap();
    let started = tokio::time::Instant::now();
    let error = permits.acquire().await.err().unwrap();
    assert_eq!(started.elapsed(), Duration::from_millis(700));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
}

#[tokio::test(start_paused = true)]
async fn a_permit_freed_inside_the_wait_is_granted_to_the_waiter() {
    let permits = Arc::new(StreamedFilePermits::new(1, Duration::from_secs(5)));
    let held = permits.acquire().await.unwrap();
    let waiter = {
        let permits = permits.clone();
        tokio::spawn(async move { permits.acquire().await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop(held);
    assert!(waiter.await.unwrap().is_ok());
}

#[tokio::test]
async fn the_peak_is_the_most_held_at_once_and_survives_release() {
    let permits = StreamedFilePermits::new(4, Duration::from_millis(10));
    let a = permits.acquire().await.unwrap();
    let b = permits.acquire().await.unwrap();
    let c = permits.acquire().await.unwrap();
    assert_eq!((permits.held(), permits.peak()), (3, 3));
    drop((a, b, c));
    assert_eq!((permits.held(), permits.peak()), (0, 3));
    let _d = permits.acquire().await.unwrap();
    assert_eq!((permits.held(), permits.peak()), (1, 3));
}

/// The descriptor budget and the handler pool read one constant, so the raise and the bound
/// cannot disagree.
#[test]
fn the_production_pool_is_the_descriptor_budget_term() {
    let permits = StreamedFilePermits::production();
    assert_eq!(permits.capacity(), STREAMED_FILE_PERMITS);
    assert_eq!(permits.held(), 0);
}
