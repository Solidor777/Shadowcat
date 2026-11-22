//! Tests for the IGD flow against a loopback fake gateway and SSDP responder.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;
use crate::portmap::test_support::{
    description, http_response, spawn_http, spawn_ssdp, ssdp_reply, FakeHttp,
};

/// The service type the fake gateway reports.
const SERVICE: &str = "urn:schemas-upnp-org:service:WANIPConnection:2";

/// Counts loopback as a LAN so the fakes pass the trust check.
fn accept_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

/// A fake IGD serving `/desc.xml` and answering SOAP through `soap`.
async fn fake_igd(soap: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> FakeHttp {
    spawn_http(move |request| {
        if request.starts_with("GET /desc.xml ") {
            return Some(http_response(200, &description(SERVICE, "/ctl")));
        }
        let (status, body) = soap(request);
        Some(http_response(status, &body))
    })
    .await
}

/// Runs discovery against a fake SSDP responder that points at `igd`, returning the first gateway.
async fn gateway_for(igd: &FakeHttp) -> IgdGateway {
    let location = format!("http://{}/desc.xml", igd.addr);
    let ssdp = spawn_ssdp(vec![ssdp_reply(&location)]).await;
    let found = discover(ssdp, Duration::from_millis(300), accept_loopback)
        .await
        .unwrap();
    found.into_iter().next().unwrap()
}

/// A SOAP fault body with `code`.
fn fault(code: u32) -> (u16, String) {
    (
        500,
        format!("<Fault><UPnPError><errorCode>{code}</errorCode></UPnPError></Fault>"),
    )
}

/// A successful empty SOAP reply.
fn ok() -> (u16, String) {
    (200, "<r/>".to_owned())
}

#[tokio::test]
async fn discovery_walks_ssdp_description_and_control_url() {
    let igd = fake_igd(|_| ok()).await;
    let gateway = gateway_for(&igd).await;
    assert!(gateway.responder.is_loopback());
    assert_eq!(gateway.control.addr, igd.addr);
    assert_eq!(gateway.control.path, "/ctl");
    assert_eq!(gateway.service_type, SERVICE);
}

#[tokio::test]
async fn external_address_is_read_from_the_reply() {
    let igd = fake_igd(|request| {
        assert!(request.contains("GetExternalIPAddress"));
        (
            200,
            "<r><NewExternalIPAddress>198.51.100.7</NewExternalIPAddress></r>".to_owned(),
        )
    })
    .await;
    let address = gateway_for(&igd).await.external_address().await.unwrap();
    assert_eq!(address, "198.51.100.7".parse::<IpAddr>().unwrap());
}

#[tokio::test]
async fn add_mapping_sends_every_argument() {
    let igd = fake_igd(|_| ok()).await;
    let gateway = gateway_for(&igd).await;
    let mapping = gateway.add_mapping(8080, 9090).await.unwrap();
    assert_eq!(
        mapping,
        IgdMapping {
            external_port: 9090,
            lease_secs: 3600
        }
    );
    let request = igd.requests.lock().unwrap().last().unwrap().clone();
    for needle in [
        "<NewExternalPort>9090</NewExternalPort>",
        "<NewInternalPort>8080</NewInternalPort>",
        "<NewProtocol>TCP</NewProtocol>",
        "<NewInternalClient>127.0.0.1</NewInternalClient>",
        "<NewEnabled>1</NewEnabled>",
        "<NewPortMappingDescription>Shadowcat</NewPortMappingDescription>",
        "<NewLeaseDuration>3600</NewLeaseDuration>",
    ] {
        assert!(request.contains(needle), "missing {needle}");
    }
}

#[tokio::test]
async fn code_725_retries_once_with_a_permanent_lease() {
    let igd = fake_igd(|request| {
        if request.contains("<NewLeaseDuration>0<") {
            ok()
        } else {
            fault(725)
        }
    })
    .await;
    let mapping = gateway_for(&igd)
        .await
        .add_mapping(8080, 8080)
        .await
        .unwrap();
    assert_eq!(
        mapping,
        IgdMapping {
            external_port: 8080,
            lease_secs: 0
        }
    );
}

#[tokio::test]
async fn a_second_725_is_an_error() {
    let igd = fake_igd(|_| fault(725)).await;
    assert_eq!(
        gateway_for(&igd).await.add_mapping(8080, 8080).await,
        Err(IgdError::Fault(725))
    );
}

#[tokio::test]
async fn code_718_moves_to_the_next_free_port() {
    let igd = fake_igd(|request| {
        if request.contains("<NewExternalPort>8080<") || request.contains("<NewExternalPort>8081<")
        {
            fault(718)
        } else {
            ok()
        }
    })
    .await;
    let mapping = gateway_for(&igd)
        .await
        .add_mapping(8080, 8080)
        .await
        .unwrap();
    assert_eq!(mapping.external_port, 8082);
}

#[tokio::test]
async fn conflicts_past_the_window_give_up_with_the_fault() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let igd = fake_igd(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        fault(718)
    })
    .await;
    let result = gateway_for(&igd).await.add_mapping(8080, 8080).await;
    assert_eq!(result, Err(IgdError::Fault(718)));
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        usize::from(CONFLICT_WINDOW) + 1
    );
}

#[tokio::test]
async fn conflict_search_stops_at_the_top_of_the_port_range() {
    let igd = fake_igd(|_| fault(718)).await;
    let result = gateway_for(&igd).await.add_mapping(8080, u16::MAX).await;
    assert_eq!(result, Err(IgdError::Fault(718)));
}

#[tokio::test]
async fn other_faults_are_surfaced_without_retry() {
    let igd = fake_igd(|_| fault(729)).await;
    assert_eq!(
        gateway_for(&igd).await.add_mapping(8080, 8080).await,
        Err(IgdError::Fault(729))
    );
    assert_eq!(
        igd.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.contains("AddPortMapping"))
            .count(),
        1
    );
}

#[tokio::test]
async fn delete_mapping_names_the_external_port() {
    let igd = fake_igd(|_| ok()).await;
    gateway_for(&igd).await.delete_mapping(9090).await.unwrap();
    let request = igd.requests.lock().unwrap().last().unwrap().clone();
    assert!(request.contains("#DeletePortMapping"));
    assert!(request.contains("<NewExternalPort>9090</NewExternalPort>"));
}

#[tokio::test]
async fn location_naming_another_host_is_untrusted_and_never_fetched() {
    let ssdp = spawn_ssdp(vec![ssdp_reply("http://10.9.9.9:80/desc.xml")]).await;
    let result = discover(ssdp, Duration::from_millis(300), accept_loopback).await;
    assert!(matches!(result, Err(IgdError::Untrusted)));
}

#[tokio::test]
async fn control_url_naming_another_host_is_untrusted() {
    let igd = spawn_http(|request| {
        request
            .starts_with("GET /desc.xml ")
            .then(|| http_response(200, &description(SERVICE, "http://10.9.9.9/ctl")))
    })
    .await;
    let ssdp = spawn_ssdp(vec![ssdp_reply(&format!("http://{}/desc.xml", igd.addr))]).await;
    let result = discover(ssdp, Duration::from_millis(300), accept_loopback).await;
    assert!(matches!(result, Err(IgdError::Untrusted)));
}

#[tokio::test]
async fn production_trust_rejects_a_loopback_gateway() {
    let igd = fake_igd(|_| ok()).await;
    let ssdp = spawn_ssdp(vec![ssdp_reply(&format!("http://{}/desc.xml", igd.addr))]).await;
    let result = discover(
        ssdp,
        Duration::from_millis(300),
        crate::portmap::soap::is_lan_address,
    )
    .await;
    assert!(matches!(result, Err(IgdError::Untrusted)));
}

#[tokio::test]
async fn no_responder_is_no_response() {
    let ssdp = spawn_ssdp(Vec::new()).await;
    let result = discover(ssdp, Duration::from_millis(200), accept_loopback).await;
    assert!(matches!(result, Err(IgdError::NoResponse)));
}

#[tokio::test]
async fn a_description_without_a_wan_service_is_refused() {
    let igd = spawn_http(|_| Some(http_response(200, "<root/>"))).await;
    let ssdp = spawn_ssdp(vec![ssdp_reply(&format!("http://{}/desc.xml", igd.addr))]).await;
    let result = discover(ssdp, Duration::from_millis(300), accept_loopback).await;
    assert!(matches!(result, Err(IgdError::NoService)));
}
