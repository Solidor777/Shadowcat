//! Tests for the XML scanner, description walk, endpoint trust and the bounded HTTP exchange.

use super::*;
use crate::portmap::test_support::{description, http_response, spawn_http};

/// A WANIPConnection v1 service type.
const IP1: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";

/// Accepts loopback, so the fakes on 127.0.0.1 count as a LAN.
fn accept_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

#[test]
fn nested_v1_description_yields_service_and_control() {
    let found = parse_description(&description(IP1, "/ctl/IPConn")).unwrap();
    assert_eq!(found.service_type, IP1);
    assert_eq!(found.control_url, "/ctl/IPConn");
    assert_eq!(found.url_base, None);
}

#[test]
fn v2_service_type_is_used_verbatim() {
    let v2 = "urn:schemas-upnp-org:service:WANIPConnection:2";
    assert_eq!(
        parse_description(&description(v2, "/c"))
            .unwrap()
            .service_type,
        v2
    );
}

#[test]
fn ppp_service_is_the_fallback_and_ip_wins_when_both_exist() {
    let ppp = "urn:schemas-upnp-org:service:WANPPPConnection:1";
    assert_eq!(
        parse_description(&description(ppp, "/p"))
            .unwrap()
            .service_type,
        ppp
    );
    let both = format!(
        "<root><service><serviceType>{ppp}</serviceType><controlURL>/p</controlURL></service>\
         <service><serviceType>{IP1}</serviceType><controlURL>/i</controlURL></service></root>"
    );
    let chosen = parse_description(&both).unwrap();
    assert_eq!(
        (chosen.service_type.as_str(), chosen.control_url.as_str()),
        (IP1, "/i")
    );
}

#[test]
fn url_base_and_prefixed_tags_are_read() {
    let xml = format!(
        "<d:root xmlns:d=\"x\"><d:URLBase>http://192.168.1.1:49152/</d:URLBase><d:service>\
         <d:serviceType>{IP1}</d:serviceType><d:controlURL>/c</d:controlURL></d:service></d:root>"
    );
    let found = parse_description(&xml).unwrap();
    assert_eq!(found.url_base.as_deref(), Some("http://192.168.1.1:49152/"));
}

#[test]
fn no_wan_service_is_an_error() {
    let xml = "<root><service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/x</controlURL></service></root>";
    assert_eq!(parse_description(xml), Err(IgdError::NoService));
}

#[test]
fn service_type_with_markup_characters_is_not_chosen() {
    let xml = "<root><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1\"&lt;x</serviceType><controlURL>/x</controlURL></service></root>";
    assert_eq!(parse_description(xml), Err(IgdError::NoService));
}

#[test]
fn doctype_entity_and_cdata_are_refused() {
    for xml in [
        "<!DOCTYPE r [<!ENTITY a \"b\">]><r/>",
        "<r><![CDATA[x]]></r>",
        "<r><!ENTITY a \"b\"></r>",
    ] {
        assert_eq!(parse_description(xml), Err(IgdError::ForbiddenXml), "{xml}");
        assert_eq!(element_text(xml, "r"), Err(IgdError::ForbiddenXml), "{xml}");
    }
}

#[test]
fn oversized_document_is_refused_before_parsing() {
    let xml = format!("<r>{}</r>", "a".repeat(MAX_DOCUMENT_BYTES));
    assert_eq!(parse_description(&xml), Err(IgdError::TooLarge));
    assert_eq!(fault_code(&xml), Err(IgdError::TooLarge));
}

#[test]
fn comments_are_skipped_and_unterminated_ones_fail() {
    assert_eq!(
        element_text("<r><!-- a > b --><a>1</a></r>", "a"),
        Ok(Some("1"))
    );
    assert!(matches!(
        element_text("<r><!-- open", "a"),
        Err(IgdError::Malformed(_))
    ));
    assert!(matches!(
        element_text("<r", "a"),
        Err(IgdError::Malformed(_))
    ));
}

#[test]
fn fault_bodies_carry_725_and_718() {
    for code in [725u32, 718] {
        let body = format!(
            "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{code}</errorCode>\
             <errorDescription>x</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
        );
        assert_eq!(fault_code(&body), Ok(Some(code)));
    }
    assert_eq!(fault_code("<errorCode>abc</errorCode>"), Ok(None));
}

#[test]
fn malformed_xml_never_panics() {
    let sample = description(IP1, "/c");
    for end in 0..sample.len() {
        let _ = parse_description(&sample[..end]);
        let _ = element_text(&sample[..end], "controlURL");
    }
    for junk in [
        "",
        "<",
        ">",
        "</>",
        "<//>",
        "<a/",
        "&&&",
        "<a><b></a></b>",
        "\u{0}",
    ] {
        let _ = parse_description(junk);
    }
}

#[test]
fn envelope_names_the_service_and_action() {
    let body = soap_envelope(
        IP1,
        "AddPortMapping",
        &[("NewExternalPort", "80".to_owned())],
    );
    assert!(body.contains(&format!("<u:AddPortMapping xmlns:u=\"{IP1}\">")));
    assert!(body.contains("<NewExternalPort>80</NewExternalPort>"));
}

#[test]
fn lan_address_table() {
    for yes in [
        "10.1.2.3",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.1.1",
        "fd00::1",
        "fe80::1",
    ] {
        assert!(is_lan_address(yes.parse().unwrap()), "{yes}");
    }
    for no in [
        "127.0.0.1",
        "8.8.8.8",
        "100.64.0.1",
        "::1",
        "2001:db8::1",
        "0.0.0.0",
    ] {
        assert!(!is_lan_address(no.parse().unwrap()), "{no}");
    }
}

#[test]
fn trust_requires_http_same_ip_and_lan() {
    let responder: IpAddr = "192.168.1.1".parse().unwrap();
    let ok = trusted_endpoint(
        &Url::parse("http://192.168.1.1:5000/a?b=1").unwrap(),
        responder,
        is_lan_address,
    )
    .unwrap();
    assert_eq!(ok.addr, "192.168.1.1:5000".parse().unwrap());
    assert_eq!(ok.path, "/a?b=1");
    for bad in [
        "http://192.168.1.9/x",
        "https://192.168.1.1/x",
        "http://router.example/x",
        "http://[::1]/x",
        "ftp://192.168.1.1/x",
    ] {
        assert_eq!(
            trusted_endpoint(&Url::parse(bad).unwrap(), responder, is_lan_address),
            Err(IgdError::Untrusted),
            "{bad}"
        );
    }
}

#[test]
fn public_or_loopback_responder_is_untrusted_even_when_hosts_match() {
    for ip in ["8.8.8.8", "127.0.0.1"] {
        let url = Url::parse(&format!("http://{ip}/x")).unwrap();
        assert_eq!(
            trusted_endpoint(&url, ip.parse().unwrap(), is_lan_address),
            Err(IgdError::Untrusted),
            "{ip}"
        );
    }
}

/// Builds an endpoint for the fake server.
fn endpoint(addr: SocketAddr) -> Endpoint {
    Endpoint {
        addr,
        path: "/x".to_owned(),
    }
}

#[tokio::test]
async fn http_request_reads_a_capped_reply() {
    let fake = spawn_http(|_| Some(http_response(200, "hello"))).await;
    let reply = http_request(
        &endpoint(fake.addr),
        "GET",
        &[],
        b"",
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!((reply.status, reply.body.as_slice()), (200, &b"hello"[..]));
}

#[tokio::test]
async fn http_request_refuses_an_oversized_body() {
    let big = "a".repeat(MAX_DOCUMENT_BYTES + 1);
    let fake = spawn_http(move |_| Some(http_response(200, &big))).await;
    let result = http_request(
        &endpoint(fake.addr),
        "GET",
        &[],
        b"",
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(result, Err(IgdError::TooLarge));
}

#[tokio::test]
async fn http_request_refuses_an_oversized_body_with_no_length() {
    let big = format!(
        "HTTP/1.0 200 OK\r\n\r\n{}",
        "a".repeat(MAX_DOCUMENT_BYTES + 100)
    );
    let fake = spawn_http(move |_| Some(big.clone().into_bytes())).await;
    let result = http_request(
        &endpoint(fake.addr),
        "GET",
        &[],
        b"",
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(result, Err(IgdError::TooLarge));
}

#[tokio::test]
async fn http_request_refuses_chunked_and_non_http_replies() {
    let chunked = spawn_http(|_| {
        Some(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec())
    })
    .await;
    assert!(matches!(
        http_request(
            &endpoint(chunked.addr),
            "GET",
            &[],
            b"",
            Duration::from_secs(2)
        )
        .await,
        Err(IgdError::Malformed(_))
    ));
    let junk = spawn_http(|_| Some(b"SSH-2.0-x\r\n\r\n".to_vec())).await;
    assert!(matches!(
        http_request(
            &endpoint(junk.addr),
            "GET",
            &[],
            b"",
            Duration::from_secs(2)
        )
        .await,
        Err(IgdError::Malformed(_))
    ));
}

#[tokio::test]
async fn http_request_times_out_on_a_silent_server() {
    let fake = spawn_http(|_| None).await;
    let result = http_request(
        &endpoint(fake.addr),
        "GET",
        &[],
        b"",
        Duration::from_millis(200),
    )
    .await;
    assert_eq!(result, Err(IgdError::Timeout));
}

#[tokio::test]
async fn soap_call_sends_action_header_and_maps_a_fault() {
    let fake = spawn_http(|_| {
        Some(http_response(
            500,
            "<Fault><UPnPError><errorCode>725</errorCode></UPnPError></Fault>",
        ))
    })
    .await;
    let result = soap_call(&endpoint(fake.addr), IP1, "AddPortMapping", &[]).await;
    assert_eq!(result, Err(IgdError::Fault(725)));
    let request = fake.requests.lock().unwrap()[0].clone();
    assert!(request.starts_with("POST /x HTTP/1.0\r\n"));
    assert!(request.contains(&format!("SOAPAction: \"{IP1}#AddPortMapping\"")));
    assert!(request.contains("<u:AddPortMapping"));
}

#[tokio::test]
async fn soap_call_maps_a_plain_error_status() {
    let fake = spawn_http(|_| Some(http_response(404, "nope"))).await;
    assert_eq!(
        soap_call(&endpoint(fake.addr), IP1, "X", &[]).await,
        Err(IgdError::Status(404))
    );
}

#[test]
fn accept_loopback_helper_matches_trust_for_the_fakes() {
    let url = Url::parse("http://127.0.0.1:9/x").unwrap();
    assert!(trusted_endpoint(&url, "127.0.0.1".parse().unwrap(), accept_loopback).is_ok());
}
