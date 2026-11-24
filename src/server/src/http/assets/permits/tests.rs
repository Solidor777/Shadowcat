//! These tests cover the permit pool: a bounded wait that ends in a 503 with `Retry-After`,
//! release on drop, and a peak gauge that never goes down.

use super::*;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

#[tokio::test(start_paused = true)]
async fn an_exhausted_pool_refuses_after_exactly_its_wait() {
    let permits = StreamedFilePermits::new(1, Duration::from_millis(700));
    let _held = permits.acquire(Uuid::nil()).await.unwrap();
    let started = tokio::time::Instant::now();
    let error = permits.acquire(Uuid::nil()).await.err().unwrap();
    assert_eq!(started.elapsed(), Duration::from_millis(700));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
}

#[tokio::test(start_paused = true)]
async fn a_permit_freed_inside_the_wait_is_granted_to_the_waiter() {
    let permits = Arc::new(StreamedFilePermits::new(1, Duration::from_secs(5)));
    let held = permits.acquire(Uuid::nil()).await.unwrap();
    let waiter = {
        let permits = permits.clone();
        tokio::spawn(async move { permits.acquire(Uuid::nil()).await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop(held);
    assert!(waiter.await.unwrap().is_ok());
}

/// One user cannot take more than their share of the pool, so another user still gets a permit.
/// Fails if `acquire` ignores the user: the first user would then drain all four permits.
#[tokio::test(start_paused = true)]
async fn one_user_cannot_hold_more_than_their_share_of_the_pool() {
    let permits = StreamedFilePermits::new(4, Duration::from_millis(50));
    let hog = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let _only = permits.acquire(hog).await.unwrap();
    assert!(
        permits.acquire(hog).await.is_err(),
        "a pool of four gives one user one permit"
    );
    assert_eq!(permits.held(), 1, "the refused request holds nothing");
    let _theirs = permits.acquire(other).await.unwrap();
    assert_eq!(permits.held(), 2);
}

/// A share permit frees with its holder, so the same user can ask again.
#[tokio::test(start_paused = true)]
async fn a_dropped_permit_returns_the_users_share() {
    let permits = StreamedFilePermits::new(4, Duration::from_millis(50));
    let user = Uuid::from_u128(1);
    drop(permits.acquire(user).await.unwrap());
    assert!(permits.acquire(user).await.is_ok());
}

/// The descriptor budget and the handler pool read one constant, so the raise and the bound
/// cannot disagree.
#[test]
fn the_production_pool_is_the_descriptor_budget_term() {
    let permits = StreamedFilePermits::production();
    assert_eq!(permits.capacity(), STREAMED_FILE_PERMITS);
    assert_eq!(permits.held(), 0);
}
