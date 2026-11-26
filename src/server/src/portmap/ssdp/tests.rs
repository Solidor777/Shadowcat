//! Tests for the SSDP request, reply parsing and the bounded collection window.

use super::*;
use crate::portmap::test_support::{spawn_ssdp, ssdp_reply};

#[test]
fn request_carries_the_discover_headers() {
    let text = search_request(SEARCH_TARGETS[1]);
    assert!(text.contains("MAN: \"ssdp:discover\"\r\n"));
    assert!(text.contains("MX: 2\r\n"));
    assert!(text.ends_with("\r\n\r\n"));
}

#[test]
fn reply_table() {
    let ok = ssdp_reply("http://192.168.1.1:5000/d");
    assert_eq!(
        parse_reply(&ok).as_deref(),
        Some("http://192.168.1.1:5000/d")
    );
    let lower = b"HTTP/1.1 200 OK\r\nst: urn:schemas-upnp-org:device:InternetGatewayDevice:2\r\nlocation: http://192.168.1.1/d\r\n\r\n";
    assert!(parse_reply(lower).is_some());
    let cases: [&[u8]; 6] = [
        b"HTTP/1.1 500 X\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: http://a/\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nST: upnp:rootdevice\r\nLOCATION: http://a/\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n",
        b"NOTIFY * HTTP/1.1\r\n\r\n",
        b"\xff\xfe",
        b"",
    ];
    for case in cases {
        assert_eq!(
            parse_reply(case),
            None,
            "{:?}",
            String::from_utf8_lossy(case)
        );
    }
}

#[test]
fn oversized_reply_is_dropped() {
    let mut reply = ssdp_reply("http://192.168.1.1/d");
    reply.extend(std::iter::repeat_n(b' ', MAX_REPLY_BYTES));
    assert_eq!(parse_reply(&reply), None);
}

#[test]
fn reply_parser_never_panics_on_any_prefix() {
    let reply = ssdp_reply("http://192.168.1.1/d");
    for end in 0..=reply.len() {
        let _ = parse_reply(&reply[..end]);
    }
}

#[tokio::test]
async fn one_responder_is_kept_once_across_both_searches() {
    let fake = spawn_ssdp(vec![ssdp_reply("http://127.0.0.1:1/d")]).await;
    let found = discover_responders(fake, Duration::from_millis(300)).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].location, "http://127.0.0.1:1/d");
    assert!(found[0].ip.is_loopback());
}

#[tokio::test]
async fn silence_returns_no_responders_when_the_window_ends() {
    let fake = spawn_ssdp(Vec::new()).await;
    assert!(discover_responders(fake, Duration::from_millis(200))
        .await
        .is_empty());
}

#[tokio::test]
async fn junk_replies_are_ignored() {
    let fake = spawn_ssdp(vec![b"garbage".to_vec()]).await;
    assert!(discover_responders(fake, Duration::from_millis(200))
        .await
        .is_empty());
}
