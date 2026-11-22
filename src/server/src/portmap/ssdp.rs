//! SSDP discovery of Internet Gateway Devices: one `M-SEARCH` per device version, then a bounded
//! collection window.
//!
//! Replies are untrusted datagrams. Each is capped at [`MAX_REPLY_BYTES`], at most
//! [`MAX_RESPONDERS`] distinct source addresses are kept, and the collection window is fixed, so
//! a LAN flood cannot grow memory or extend the wait.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

/// The SSDP multicast group and port.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::ssdp::SSDP_GROUP.port(), 1900);
/// ```
pub const SSDP_GROUP: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(239, 255, 255, 250)), 1900);

/// How long replies are collected after the searches are sent.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// assert_eq!(shadowcat::portmap::ssdp::COLLECT_WINDOW, Duration::from_secs(3));
/// ```
pub const COLLECT_WINDOW: Duration = Duration::from_secs(3);

/// Largest accepted reply datagram.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::ssdp::MAX_REPLY_BYTES, 2048);
/// ```
pub const MAX_REPLY_BYTES: usize = 2048;

/// Most distinct responders kept.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::ssdp::MAX_RESPONDERS, 8);
/// ```
pub const MAX_RESPONDERS: usize = 8;

/// Search targets: InternetGatewayDevice versions 1 and 2.
const SEARCH_TARGETS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
];

/// Prefix every accepted reply's `ST` must carry.
const IGD_PREFIX: &str = "urn:schemas-upnp-org:device:InternetGatewayDevice:";

/// A device that answered the search.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::ssdp::Responder;
/// let responder = Responder { ip: "192.168.1.1".parse().unwrap(), location: "http://192.168.1.1/d".into() };
/// assert_eq!(responder.location, "http://192.168.1.1/d");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Responder {
    /// Source address of the reply; every later URL must name this address.
    pub ip: IpAddr,
    /// The `LOCATION` header: where the device description lives.
    pub location: String,
}

/// Builds the `M-SEARCH` datagram for one search target.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::ssdp::search_request;
/// let text = search_request("urn:x");
/// assert!(text.starts_with("M-SEARCH * HTTP/1.1\r\n"));
/// assert!(text.contains("ST: urn:x\r\n"));
/// ```
pub fn search_request(search_target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {search_target}\r\n\r\n"
    )
}

/// Parses one reply datagram into its `LOCATION`. Returns `None` for anything that is not a
/// `200` reply for an InternetGatewayDevice with a location.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::ssdp::parse_reply;
/// let reply = b"HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLocation: http://192.168.1.1:5000/d\r\n\r\n";
/// assert_eq!(parse_reply(reply).as_deref(), Some("http://192.168.1.1:5000/d"));
/// assert_eq!(parse_reply(b"NOTIFY * HTTP/1.1\r\n\r\n"), None);
/// ```
pub fn parse_reply(datagram: &[u8]) -> Option<String> {
    if datagram.len() > MAX_REPLY_BYTES {
        return None;
    }
    let text = std::str::from_utf8(datagram).ok()?;
    let mut lines = text.split("\r\n");
    let status = lines.next()?;
    let mut status_parts = status.split_whitespace();
    if !status_parts.next()?.starts_with("HTTP/1.") || status_parts.next()? != "200" {
        return None;
    }
    let mut location = None;
    let mut is_igd = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.trim().eq_ignore_ascii_case("location") {
            location = Some(value.to_owned());
        } else if name.trim().eq_ignore_ascii_case("st") {
            is_igd = value.starts_with(IGD_PREFIX);
        }
    }
    if is_igd {
        location
    } else {
        None
    }
}

/// Sends a search for each IGD version to `target` and collects distinct responders for
/// `window`. Socket failures end collection early with whatever was gathered.
///
/// # Examples
///
/// ```no_run
/// use shadowcat::portmap::ssdp::{discover_responders, COLLECT_WINDOW, SSDP_GROUP};
/// # async fn demo() {
/// let responders = discover_responders(SSDP_GROUP, COLLECT_WINDOW).await;
/// # let _ = responders;
/// # }
/// ```
pub async fn discover_responders(target: SocketAddr, window: Duration) -> Vec<Responder> {
    let bind: SocketAddr = if target.is_ipv4() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0)
    };
    let Ok(socket) = UdpSocket::bind(bind).await else {
        return Vec::new();
    };
    for search_target in SEARCH_TARGETS {
        if socket
            .send_to(search_request(search_target).as_bytes(), target)
            .await
            .is_err()
        {
            return Vec::new();
        }
    }
    let Some(end) = Instant::now().checked_add(window) else {
        return Vec::new();
    };
    let mut responders: Vec<Responder> = Vec::new();
    // One spare byte distinguishes a datagram of exactly the cap from a truncated larger one.
    let mut buf = [0u8; 2049];
    while responders.len() < MAX_RESPONDERS {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(Ok((len, source))) =
            tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await
        else {
            break;
        };
        let ip = source.ip();
        if responders.iter().any(|known| known.ip == ip) {
            continue;
        }
        if let Some(location) = buf.get(..len).and_then(parse_reply) {
            responders.push(Responder { ip, location });
        }
    }
    responders
}

#[cfg(test)]
mod tests;
