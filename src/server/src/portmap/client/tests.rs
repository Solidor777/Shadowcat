//! Tests for the NAT-PMP and PCP exchanges against a loopback fake gateway.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::portmap::test_support::{
    natpmp_address_reply, natpmp_map_reply, natpmp_refusal, pcp_map_reply, spawn_udp, Reply,
};

/// A short schedule so silence resolves in milliseconds.
const FAST: RetryPolicy = RetryPolicy {
    initial: Duration::from_millis(30),
    attempts: 3,
};

#[tokio::test]
async fn natpmp_address_and_map_round_trip() {
    let gateway = spawn_udp(|request| match request.get(1) {
        Some(0) => vec![Reply::genuine(natpmp_address_reply(77, [203, 0, 113, 9]))],
        Some(2) => vec![Reply::genuine(natpmp_map_reply(78, 8080, 9090, 3600))],
        _ => Vec::new(),
    })
    .await;
    let (address, epoch) = natpmp_external_address(gateway.addr, FAST).await.unwrap();
    assert_eq!((address, epoch), (Ipv4Addr::new(203, 0, 113, 9), 77));
    let grant = natpmp_map(gateway.addr, 8080, 8080, 3600, FAST)
        .await
        .unwrap();
    assert_eq!(
        grant,
        PmpGrant {
            external_port: 9090,
            lifetime_secs: 3600,
            epoch: 78
        }
    );
}

#[tokio::test]
async fn a_grant_for_another_internal_port_is_a_mismatch() {
    let gateway = spawn_udp(|_| vec![Reply::genuine(natpmp_map_reply(1, 7777, 9090, 3600))]).await;
    assert_eq!(
        natpmp_map(gateway.addr, 8080, 8080, 3600, FAST).await,
        Err(ClientError::Mismatch)
    );
}

#[tokio::test]
async fn a_refusal_ends_the_exchange_without_retransmitting() {
    let seen = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&seen);
    let gateway = spawn_udp(move |request| {
        counted.fetch_add(1, Ordering::SeqCst);
        vec![Reply::genuine(natpmp_refusal(request, 2))]
    })
    .await;
    let result = natpmp_map(gateway.addr, 8080, 8080, 3600, FAST).await;
    assert_eq!(result, Err(ClientError::Refused(CodecError::ResultCode(2))));
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn junk_before_the_real_reply_is_ignored() {
    let gateway = spawn_udp(|_| {
        vec![
            Reply::genuine(vec![0xde, 0xad]),
            Reply::genuine(natpmp_map_reply(1, 8080, 8080, 60)),
        ]
    })
    .await;
    let grant = natpmp_map(gateway.addr, 8080, 8080, 3600, FAST)
        .await
        .unwrap();
    assert_eq!(grant.lifetime_secs, 60);
}

#[tokio::test]
async fn a_reply_from_another_source_is_dropped() {
    let gateway = spawn_udp(|_| {
        vec![Reply {
            bytes: natpmp_map_reply(1, 8080, 8080, 3600),
            spoofed: true,
        }]
    })
    .await;
    assert_eq!(
        natpmp_map(gateway.addr, 8080, 8080, 3600, FAST).await,
        Err(ClientError::Timeout)
    );
}

#[tokio::test]
async fn silence_retransmits_the_full_schedule_then_times_out() {
    let gateway = spawn_udp(|_| Vec::new()).await;
    assert_eq!(
        natpmp_map(gateway.addr, 8080, 8080, 3600, FAST).await,
        Err(ClientError::Timeout)
    );
    assert_eq!(gateway.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_closed_port_is_a_timeout_error() {
    let closed = {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.local_addr().unwrap()
    };
    assert_eq!(
        natpmp_map(closed, 8080, 8080, 3600, FAST).await,
        Err(ClientError::Timeout)
    );
}

#[tokio::test]
async fn natpmp_delete_sends_lifetime_zero() {
    let gateway = spawn_udp(|_| vec![Reply::genuine(natpmp_map_reply(1, 8080, 0, 0))]).await;
    natpmp_delete(gateway.addr, 8080, FAST).await.unwrap();
    let request = gateway.requests.lock().unwrap()[0].clone();
    assert_eq!(request, natpmp_delete_request(8080));
}

#[tokio::test]
async fn pcp_map_uses_the_nonce_and_reports_the_address() {
    let gateway = spawn_udp(|request| {
        vec![Reply::genuine(pcp_map_reply(
            request,
            0,
            1800,
            5,
            9090,
            Ipv4Addr::new(203, 0, 113, 9),
        ))]
    })
    .await;
    let grant = pcp_map(gateway.addr, [4; 12], 8080, 8080, 3600, FAST)
        .await
        .unwrap();
    assert_eq!(grant.external_port, 9090);
    assert_eq!(grant.lifetime_secs, 1800);
    assert_eq!(
        grant.external_address,
        "203.0.113.9".parse::<IpAddr>().unwrap()
    );
    let request = gateway.requests.lock().unwrap()[0].clone();
    assert_eq!(&request[24..36], &[4; 12]);
}

#[tokio::test]
async fn a_pcp_reply_with_another_nonce_is_ignored() {
    let gateway = spawn_udp(|request| {
        let mut reply = pcp_map_reply(request, 0, 1800, 5, 9090, Ipv4Addr::new(203, 0, 113, 9));
        reply[24..36].copy_from_slice(&[9; 12]);
        vec![Reply::genuine(reply)]
    })
    .await;
    assert_eq!(
        pcp_map(gateway.addr, [4; 12], 8080, 8080, 3600, FAST).await,
        Err(ClientError::Timeout)
    );
}

#[tokio::test]
async fn pcp_refusal_is_reported() {
    let gateway = spawn_udp(|request| {
        vec![Reply::genuine(pcp_map_reply(
            request,
            2,
            0,
            0,
            0,
            Ipv4Addr::UNSPECIFIED,
        ))]
    })
    .await;
    assert_eq!(
        pcp_map(gateway.addr, [4; 12], 8080, 8080, 3600, FAST).await,
        Err(ClientError::Refused(CodecError::ResultCode(2)))
    );
}

#[tokio::test]
async fn pcp_delete_sends_lifetime_zero_with_the_same_nonce() {
    let gateway = spawn_udp(|request| {
        vec![Reply::genuine(pcp_map_reply(
            request,
            0,
            0,
            5,
            0,
            Ipv4Addr::UNSPECIFIED,
        ))]
    })
    .await;
    pcp_delete(gateway.addr, [4; 12], 8080, FAST).await.unwrap();
    let request = gateway.requests.lock().unwrap()[0].clone();
    assert_eq!(&request[4..8], &[0, 0, 0, 0]);
    assert_eq!(&request[24..36], &[4; 12]);
}

#[test]
fn the_production_schedule_doubles_from_250_ms_and_stops_at_five() {
    let waits: Vec<_> = (0..6).map(|n| RetryPolicy::PRODUCTION.delay(n)).collect();
    assert_eq!(
        waits,
        vec![
            Some(Duration::from_millis(250)),
            Some(Duration::from_millis(500)),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            Some(Duration::from_secs(4)),
            None
        ]
    );
}
