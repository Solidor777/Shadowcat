//! Tests for the protocol ladder against loopback fakes.

use super::*;
use crate::portmap::test_support::{
    accept_loopback, description, http_response, natpmp_address_reply, natpmp_map_reply,
    pcp_map_reply, spawn_http, spawn_ssdp, spawn_udp, ssdp_reply, FakeUdp, Reply,
};

/// The IGD service type the fake reports.
const SERVICE: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";

/// A short NAT-PMP/PCP schedule so silence resolves in milliseconds.
const FAST: RetryPolicy = RetryPolicy {
    initial: Duration::from_millis(30),
    attempts: 2,
};

/// A closed loopback UDP port: nothing answers there.
async fn dead_udp() -> SocketAddr {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.local_addr().unwrap()
}

/// A loopback SSDP responder that never answers.
async fn silent_ssdp() -> SocketAddr {
    spawn_ssdp(Vec::new()).await
}

/// Targets aimed at the fakes.
fn targets(gateway: Option<SocketAddr>, ssdp: SocketAddr) -> Targets {
    Targets {
        gateway,
        ssdp,
        ssdp_window: Duration::from_millis(200),
        accept: accept_loopback,
        retry: FAST,
        backoff_initial: Duration::from_millis(50),
        backoff_max: Duration::from_millis(200),
        delete_deadline: Duration::from_secs(2),
    }
}

/// A fake NAT-PMP gateway granting external port 9090 for `lifetime` seconds.
async fn natpmp_gateway(lifetime: u32) -> FakeUdp {
    spawn_udp(move |request| match request.get(1) {
        Some(0) => vec![Reply::genuine(natpmp_address_reply(10, [203, 0, 113, 9]))],
        Some(2) => {
            let internal = u16::from_be_bytes([request[4], request[5]]);
            let granted = if request[8..12] == [0; 4] {
                0
            } else {
                lifetime
            };
            vec![Reply::genuine(natpmp_map_reply(
                11, internal, 9090, granted,
            ))]
        }
        _ => Vec::new(),
    })
    .await
}

/// A fake PCP gateway granting external port 9091.
async fn pcp_gateway() -> FakeUdp {
    spawn_udp(|request| {
        if request.first() == Some(&2) {
            vec![Reply::genuine(pcp_map_reply(
                request,
                0,
                3600,
                20,
                9091,
                std::net::Ipv4Addr::new(203, 0, 113, 10),
            ))]
        } else {
            Vec::new()
        }
    })
    .await
}

/// A fake IGD answering every SOAP call with success and a fixed external address.
async fn igd_gateway() -> (crate::portmap::test_support::FakeHttp, SocketAddr) {
    let igd = spawn_http(|request| {
        if request.starts_with("GET /desc.xml ") {
            return Some(http_response(200, &description(SERVICE, "/ctl")));
        }
        Some(http_response(
            200,
            "<r><NewExternalIPAddress>203.0.113.11</NewExternalIPAddress></r>",
        ))
    })
    .await;
    let ssdp = spawn_ssdp(vec![ssdp_reply(&format!("http://{}/desc.xml", igd.addr))]).await;
    (igd, ssdp)
}

#[tokio::test]
async fn natpmp_is_tried_first_and_wins() {
    let gateway = natpmp_gateway(3600).await;
    let mapping = acquire(&targets(Some(gateway.addr), silent_ssdp().await), 8080)
        .await
        .unwrap();
    assert_eq!(mapping.protocol, PortMapProtocol::NatPmp);
    assert_eq!(mapping.external_port, 9090);
    assert_eq!(
        mapping.external_address,
        Some("203.0.113.9".parse().unwrap())
    );
    assert_eq!(mapping.lease_secs, 3600);
    assert_eq!(mapping.renew_after(), Duration::from_secs(1800));
}

#[tokio::test]
async fn pcp_is_tried_when_natpmp_is_silent() {
    let gateway = pcp_gateway().await;
    let mapping = acquire(&targets(Some(gateway.addr), silent_ssdp().await), 8080)
        .await
        .unwrap();
    assert_eq!(mapping.protocol, PortMapProtocol::Pcp);
    assert_eq!(mapping.external_port, 9091);
}

#[tokio::test]
async fn upnp_is_tried_when_both_binary_protocols_are_silent() {
    let (igd, ssdp) = igd_gateway().await;
    let mapping = acquire(&targets(Some(dead_udp().await), ssdp), 8080)
        .await
        .unwrap();
    assert_eq!(mapping.protocol, PortMapProtocol::Upnp);
    assert_eq!(mapping.external_port, 8080);
    assert_eq!(
        mapping.external_address,
        Some("203.0.113.11".parse().unwrap())
    );
    assert!(igd
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.contains("AddPortMapping")));
}

#[tokio::test]
async fn every_protocol_silent_is_no_response() {
    let failure = acquire(&targets(Some(dead_udp().await), silent_ssdp().await), 8080)
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "no_response");
}

#[tokio::test]
async fn an_untrusted_upnp_endpoint_is_the_reported_cause() {
    let ssdp = spawn_ssdp(vec![ssdp_reply("http://10.9.9.9:80/desc.xml")]).await;
    let failure = acquire(&targets(Some(dead_udp().await), ssdp), 8080)
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "untrusted_endpoint");
}

#[tokio::test]
async fn a_refusal_is_reported_over_silence() {
    let gateway = spawn_udp(|request| {
        if request.get(1) == Some(&0) {
            vec![Reply::genuine(natpmp_address_reply(1, [203, 0, 113, 9]))]
        } else {
            vec![Reply::genuine(
                crate::portmap::test_support::natpmp_refusal(request, 2),
            )]
        }
    })
    .await;
    let failure = acquire(&targets(Some(gateway.addr), silent_ssdp().await), 8080)
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "refused");
}

#[tokio::test]
async fn a_zero_lifetime_grant_is_not_a_mapping() {
    let gateway = natpmp_gateway(0).await;
    let failure = acquire(&targets(Some(gateway.addr), silent_ssdp().await), 8080)
        .await
        .unwrap_err();
    assert_eq!(failure.reason, "refused");
}

#[tokio::test]
async fn renewal_keeps_the_port_and_follows_the_gateway_epoch() {
    let gateway = natpmp_gateway(3600).await;
    let t = targets(Some(gateway.addr), silent_ssdp().await);
    let first = acquire(&t, 8080).await.unwrap();
    let renewed = renew(&t, &first).await.unwrap();
    assert_eq!(renewed.external_port, 9090);
    assert_eq!(renewed.protocol, PortMapProtocol::NatPmp);
    let requests = gateway.requests.lock().unwrap();
    let last_map = requests
        .iter()
        .rev()
        .find(|r| r.get(1) == Some(&2))
        .unwrap();
    assert_eq!(u16::from_be_bytes([last_map[6], last_map[7]]), 9090);
}

#[tokio::test]
async fn renewal_against_a_restarted_gateway_still_returns_the_mapping() {
    let epochs = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1000));
    let served = std::sync::Arc::clone(&epochs);
    let gateway = spawn_udp(move |request| match request.get(1) {
        Some(0) => vec![Reply::genuine(natpmp_address_reply(1, [203, 0, 113, 9]))],
        Some(2) => {
            let epoch = served.load(std::sync::atomic::Ordering::SeqCst);
            vec![Reply::genuine(natpmp_map_reply(epoch, 8080, 9090, 3600))]
        }
        _ => Vec::new(),
    })
    .await;
    let t = targets(Some(gateway.addr), silent_ssdp().await);
    let first = acquire(&t, 8080).await.unwrap();
    epochs.store(5, std::sync::atomic::Ordering::SeqCst);
    let renewed = renew(&t, &first).await.unwrap();
    assert_eq!(renewed.external_port, 9090);
}

#[tokio::test]
async fn renewal_failure_is_reported() {
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let switch = std::sync::Arc::clone(&alive);
    let gateway = spawn_udp(move |request| {
        if !switch.load(std::sync::atomic::Ordering::SeqCst) {
            return Vec::new();
        }
        match request.get(1) {
            Some(0) => vec![Reply::genuine(natpmp_address_reply(10, [203, 0, 113, 9]))],
            Some(2) => vec![Reply::genuine(natpmp_map_reply(11, 8080, 9090, 3600))],
            _ => Vec::new(),
        }
    })
    .await;
    let t = targets(Some(gateway.addr), silent_ssdp().await);
    let first = acquire(&t, 8080).await.unwrap();
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(renew(&t, &first).await.unwrap_err().reason, "no_response");
}

#[tokio::test]
async fn delete_sends_the_protocol_delete() {
    let gateway = natpmp_gateway(3600).await;
    let t = targets(Some(gateway.addr), silent_ssdp().await);
    let mapping = acquire(&t, 8080).await.unwrap();
    delete(&t, &mapping).await;
    let requests = gateway.requests.lock().unwrap();
    let last = requests.last().unwrap();
    assert_eq!(last.get(1), Some(&2));
    assert_eq!(&last[8..12], &[0, 0, 0, 0]);
}

#[tokio::test]
async fn upnp_delete_names_the_external_port() {
    let (igd, ssdp) = igd_gateway().await;
    let t = targets(Some(dead_udp().await), ssdp);
    let mapping = acquire(&t, 8080).await.unwrap();
    delete(&t, &mapping).await;
    let requests = igd.requests.lock().unwrap();
    let last = requests.last().unwrap();
    assert!(last.contains("#DeletePortMapping"));
    assert!(last.contains("<NewExternalPort>8080</NewExternalPort>"));
}

#[test]
fn a_permanent_lease_is_rechecked_at_half_the_requested_lease() {
    let mapping = ActiveMapping {
        protocol: PortMapProtocol::Upnp,
        internal_port: 1,
        external_port: 1,
        external_address: None,
        lease_secs: 0,
        granted_at: Instant::now(),
        epoch: None,
        handle: Handle::NatPmp("127.0.0.1:5351".parse().unwrap()),
    };
    assert_eq!(mapping.renew_after(), Duration::from_secs(1800));
    assert_eq!(mapping.lease_expires_in_secs(), None);
}

#[test]
fn summarize_prefers_a_cause_then_a_missing_gateway() {
    let silent = Failure::new("no_response", "x");
    let missing = Failure::new("no_gateway", "y");
    let cause = Failure::new("refused", "z");
    assert_eq!(
        summarize(vec![silent.clone(), cause.clone()]).reason,
        "refused"
    );
    assert_eq!(summarize(vec![missing, silent]).reason, "no_gateway");
    assert_eq!(summarize(Vec::new()).reason, "no_response");
}
