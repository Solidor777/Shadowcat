//! Loopback fakes shared by the portmap tests: a scripted HTTP server and an SSDP responder.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

/// A running fake HTTP server and the request texts it has seen.
pub struct FakeHttp {
    /// Address the server listens on.
    pub addr: SocketAddr,
    /// Full text (head and body) of every request received, in order.
    pub requests: Arc<Mutex<Vec<String>>>,
}

/// Builds a complete HTTP/1.0 reply with a correct `Content-Length`.
pub fn http_response(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 {status} X\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Starts a fake HTTP server. `handler` maps a request's text to the raw bytes to send back, or
/// `None` to accept the request and never answer.
pub async fn spawn_http(
    handler: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
) -> FakeHttp {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let seen = Arc::clone(&seen);
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let Some(head_end) = text.find("\r\n\r\n") else {
                        continue;
                    };
                    let wanted = text[..head_end]
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() < head_end + 4 + wanted {
                        continue;
                    }
                    seen.lock().unwrap().push(text.clone());
                    match handler(&text) {
                        Some(reply) => {
                            let _ = stream.write_all(&reply).await;
                            return;
                        }
                        None => {
                            std::future::pending::<()>().await;
                        }
                    }
                }
            });
        }
    });
    FakeHttp { addr, requests }
}

/// Starts a fake SSDP responder that answers every datagram with each of `replies`. Returns its
/// address.
pub async fn spawn_ssdp(replies: Vec<Vec<u8>>) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        while let Ok((_, source)) = socket.recv_from(&mut buf).await {
            for reply in &replies {
                let _ = socket.send_to(reply, source).await;
            }
        }
    });
    addr
}

/// An SSDP reply datagram for an IGD at `location`.
pub fn ssdp_reply(location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: {location}\r\n\r\n"
    )
    .into_bytes()
}

/// A device description with one WAN connection service of `service_type` at `control`.
pub fn description(service_type: &str, control: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><device>\
         <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>\
         <deviceList><device><deviceList><device><serviceList><service>\
         <serviceType>{service_type}</serviceType><controlURL>{control}</controlURL>\
         </service></serviceList></device></deviceList></device></deviceList></device></root>"
    )
}
