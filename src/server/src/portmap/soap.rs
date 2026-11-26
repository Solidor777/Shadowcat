//! Bounded HTTP and SOAP plumbing for UPnP IGD: a minimal XML scanner, the device-description
//! walk, SOAP envelopes and fault parsing, endpoint trust checks and a capped HTTP/1.0 exchange.
//!
//! Every byte here comes from an unauthenticated LAN peer. The XML scanner never resolves
//! entities and rejects any `<!` construct other than a comment (DOCTYPE, ENTITY and CDATA all
//! start with it), every document and HTTP reply is capped before it is parsed, and every
//! exchange runs under a deadline.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::{Host, Url};

/// Largest XML document or HTTP body accepted from a gateway.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::soap::MAX_DOCUMENT_BYTES, 65_536);
/// ```
pub const MAX_DOCUMENT_BYTES: usize = 65_536;

/// Largest HTTP response head accepted before the body starts.
const MAX_HEAD_BYTES: usize = 8_192;

/// Deadline for one HTTP exchange (connect, send and read the whole reply).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// assert_eq!(shadowcat::portmap::soap::HTTP_DEADLINE, Duration::from_secs(5));
/// ```
pub const HTTP_DEADLINE: Duration = Duration::from_secs(5);

/// Service type prefix of a WANIPConnection service (versions 1 and 2).
const WAN_IP_PREFIX: &str = "urn:schemas-upnp-org:service:WANIPConnection:";

/// Service type prefix of a WANPPPConnection service, the fallback.
const WAN_PPP_PREFIX: &str = "urn:schemas-upnp-org:service:WANPPPConnection:";

/// Why a UPnP exchange failed.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::{fault_code, IgdError};
/// assert_eq!(fault_code("<errorCode>725</errorCode>"), Ok(Some(725)));
/// assert_eq!(fault_code("<!DOCTYPE x>"), Err(IgdError::ForbiddenXml));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IgdError {
    /// A document or reply passed its size cap.
    #[error("reply is larger than the size cap")]
    TooLarge,
    /// The XML holds a construct the scanner refuses (DOCTYPE, ENTITY, CDATA).
    #[error("XML holds a forbidden construct")]
    ForbiddenXml,
    /// The reply is not the shape the exchange requires.
    #[error("malformed reply: {0}")]
    Malformed(&'static str),
    /// The exchange did not finish before its deadline.
    #[error("exchange timed out")]
    Timeout,
    /// A socket operation failed.
    #[error("socket error: {0:?}")]
    Io(std::io::ErrorKind),
    /// The server answered an HTTP status other than 200 with no UPnP fault in the body.
    #[error("http status {0}")]
    Status(u16),
    /// The gateway answered a SOAP fault carrying this UPnP error code.
    #[error("upnp fault {0}")]
    Fault(u32),
    /// A URL named a host other than the device that answered discovery, or a non-LAN address.
    #[error("endpoint is not the discovering device on a LAN address")]
    Untrusted,
    /// The device description lists no WANIPConnection or WANPPPConnection service.
    #[error("device description has no WAN connection service")]
    NoService,
    /// No gateway answered discovery.
    #[error("no gateway answered discovery")]
    NoResponse,
}

/// One scanned XML event. Names have any namespace prefix stripped.
#[derive(Debug, PartialEq, Eq)]
enum Event<'a> {
    /// An opening tag (a self-closing tag yields an `Open` then a `Close`).
    Open(&'a str),
    /// A closing tag.
    Close(&'a str),
    /// Non-empty trimmed text between tags, with entities left unresolved.
    Text(&'a str),
}

/// Minimal XML tokenizer over a borrowed document.
struct Events<'a> {
    /// Unscanned remainder of the document.
    rest: &'a str,
    /// The `Close` owed after a self-closing tag.
    pending_close: Option<&'a str>,
}

/// Strips an XML namespace prefix.
fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl<'a> Events<'a> {
    /// Starts scanning `xml`.
    fn new(xml: &'a str) -> Self {
        Self {
            rest: xml,
            pending_close: None,
        }
    }

    /// Stops the scan and returns the error.
    fn fail(&mut self, error: IgdError) -> Option<Result<Event<'a>, IgdError>> {
        self.rest = "";
        self.pending_close = None;
        Some(Err(error))
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Result<Event<'a>, IgdError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(name) = self.pending_close.take() {
            return Some(Ok(Event::Close(name)));
        }
        loop {
            if self.rest.is_empty() {
                return None;
            }
            let Some(after_lt) = self.rest.strip_prefix('<') else {
                let end = self.rest.find('<').unwrap_or(self.rest.len());
                let text = self.rest.get(..end).unwrap_or("");
                self.rest = self.rest.get(end..).unwrap_or("");
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                return Some(Ok(Event::Text(text)));
            };
            if let Some(comment) = after_lt.strip_prefix("!--") {
                match comment.find("-->") {
                    Some(end) => {
                        // `end + 3` cannot overflow: `end` indexes a string that holds the marker.
                        self.rest = comment.get(end.saturating_add(3)..).unwrap_or("");
                        continue;
                    }
                    None => return self.fail(IgdError::Malformed("unterminated comment")),
                }
            }
            if after_lt.starts_with('!') {
                return self.fail(IgdError::ForbiddenXml);
            }
            let Some(end) = after_lt.find('>') else {
                return self.fail(IgdError::Malformed("unterminated tag"));
            };
            let inner = after_lt.get(..end).unwrap_or("");
            self.rest = after_lt.get(end.saturating_add(1)..).unwrap_or("");
            if inner.starts_with('?') {
                continue;
            }
            if let Some(closing) = inner.strip_prefix('/') {
                return Some(Ok(Event::Close(local_name(closing.trim()))));
            }
            let self_closing = inner.ends_with('/');
            let name_end = inner
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(inner.len());
            let name = local_name(inner.get(..name_end).unwrap_or(""));
            if name.is_empty() {
                return self.fail(IgdError::Malformed("empty tag name"));
            }
            if self_closing {
                self.pending_close = Some(name);
            }
            return Some(Ok(Event::Open(name)));
        }
    }
}

/// Rejects a document over [`MAX_DOCUMENT_BYTES`].
fn check_document(xml: &str) -> Result<(), IgdError> {
    if xml.len() > MAX_DOCUMENT_BYTES {
        return Err(IgdError::TooLarge);
    }
    Ok(())
}

/// Returns the text of the first element named `name`, if the document has one.
///
/// # Errors
///
/// [`IgdError::TooLarge`] past the document cap, [`IgdError::ForbiddenXml`] on a DOCTYPE, and
/// [`IgdError::Malformed`] on broken tags.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::element_text;
/// let xml = "<s:Body><u:R><NewExternalIPAddress>192.0.2.7</NewExternalIPAddress></u:R></s:Body>";
/// assert_eq!(element_text(xml, "NewExternalIPAddress"), Ok(Some("192.0.2.7")));
/// assert_eq!(element_text(xml, "Missing"), Ok(None));
/// ```
pub fn element_text<'a>(xml: &'a str, name: &str) -> Result<Option<&'a str>, IgdError> {
    check_document(xml)?;
    let mut inside = false;
    for event in Events::new(xml) {
        match event? {
            Event::Open(open) => inside = open == name,
            Event::Close(_) => inside = false,
            Event::Text(text) if inside => return Ok(Some(text)),
            Event::Text(_) => {}
        }
    }
    Ok(None)
}

/// Returns the UPnP `errorCode` of a SOAP fault body, if it carries a numeric one.
///
/// # Errors
///
/// The scanner errors of [`element_text`].
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::fault_code;
/// assert_eq!(fault_code("<UPnPError><errorCode>718</errorCode></UPnPError>"), Ok(Some(718)));
/// assert_eq!(fault_code("<ok/>"), Ok(None));
/// ```
pub fn fault_code(xml: &str) -> Result<Option<u32>, IgdError> {
    Ok(element_text(xml, "errorCode")?.and_then(|text| text.parse().ok()))
}

/// The WAN connection service chosen from a device description.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::parse_description;
/// let xml = "<root><device><serviceList><service>\
///   <serviceType>urn:schemas-upnp-org:service:WANIPConnection:2</serviceType>\
///   <controlURL>/ctl</controlURL></service></serviceList></device></root>";
/// let service = parse_description(xml).unwrap();
/// assert_eq!(service.control_url, "/ctl");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescription {
    /// The description's `URLBase`, if it has one.
    pub url_base: Option<String>,
    /// The service type exactly as the device reported it; used verbatim in every SOAP request.
    pub service_type: String,
    /// The service's `controlURL`, absolute or relative.
    pub control_url: String,
}

/// Reports whether a service type is safe to splice into a SOAP envelope.
fn service_type_is_plain(service_type: &str) -> bool {
    service_type
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
}

/// Walks a device description for a `WANIPConnection` service (any version), falling back to
/// `WANPPPConnection`. The service may sit at any depth of the device tree.
///
/// # Errors
///
/// [`IgdError::NoService`] when neither service exists, plus the scanner errors of
/// [`element_text`].
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::{parse_description, IgdError};
/// assert_eq!(parse_description("<root/>"), Err(IgdError::NoService));
/// ```
pub fn parse_description(xml: &str) -> Result<ServiceDescription, IgdError> {
    check_document(xml)?;
    let mut url_base = None;
    let mut services: Vec<(String, String)> = Vec::new();
    let mut in_service = false;
    let mut current = "";
    let mut service_type: Option<String> = None;
    let mut control_url: Option<String> = None;
    for event in Events::new(xml) {
        match event? {
            Event::Open("service") => {
                in_service = true;
                current = "";
                service_type = None;
                control_url = None;
            }
            Event::Open(name) => current = name,
            Event::Text(text) => match (in_service, current) {
                (true, "serviceType") => service_type = Some(text.to_owned()),
                (true, "controlURL") => control_url = Some(text.to_owned()),
                (false, "URLBase") => url_base = Some(text.to_owned()),
                _ => {}
            },
            Event::Close("service") => {
                if let (Some(kind), Some(control)) = (service_type.take(), control_url.take()) {
                    services.push((kind, control));
                }
                in_service = false;
                current = "";
            }
            Event::Close(_) => current = "",
        }
    }
    for prefix in [WAN_IP_PREFIX, WAN_PPP_PREFIX] {
        if let Some((kind, control)) = services
            .iter()
            .find(|(kind, _)| kind.starts_with(prefix) && service_type_is_plain(kind))
        {
            return Ok(ServiceDescription {
                url_base,
                service_type: kind.clone(),
                control_url: control.clone(),
            });
        }
    }
    Err(IgdError::NoService)
}

/// Builds the SOAP envelope for one action. `args` values are numbers, addresses or fixed words,
/// never operator or network text, so they are not escaped.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::soap_envelope;
/// let body = soap_envelope("urn:x:service:S:1", "Act", &[("NewA", "1".to_owned())]);
/// assert!(body.contains("<u:Act xmlns:u=\"urn:x:service:S:1\"><NewA>1</NewA></u:Act>"));
/// ```
pub fn soap_envelope(service_type: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut inner = String::new();
    for (name, value) in args {
        inner.push_str(&format!("<{name}>{value}</{name}>"));
    }
    format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:{action} xmlns:u=\"{service_type}\">{inner}</u:{action}></s:Body></s:Envelope>"
    )
}

/// Reports whether `ip` is a LAN address a router can plausibly sit on: RFC 1918, link-local or
/// unique-local. Loopback and public addresses are not.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::is_lan_address;
/// assert!(is_lan_address("192.168.1.1".parse().unwrap()));
/// assert!(!is_lan_address("127.0.0.1".parse().unwrap()));
/// assert!(!is_lan_address("8.8.8.8".parse().unwrap()));
/// ```
pub fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let first = v6.segments().first().copied().unwrap_or(0);
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// A validated HTTP target: the socket address and the request path.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::Endpoint;
/// let endpoint = Endpoint { addr: "192.168.1.1:80".parse().unwrap(), path: "/".to_owned() };
/// assert_eq!(endpoint.path, "/");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Where to connect; its IP equals the SSDP responder's.
    pub addr: SocketAddr,
    /// Request target: path plus query, beginning with `/`.
    pub path: String,
}

/// Accepts a discovered URL only if it is `http`, names `responder` by IP literal, and passes
/// `accept` (production passes [`is_lan_address`]). A hostile device cannot steer requests to
/// another host this way.
///
/// # Errors
///
/// [`IgdError::Untrusted`] on any mismatch.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::{is_lan_address, trusted_endpoint, IgdError};
/// use url::Url;
/// let responder = "192.168.1.1".parse().unwrap();
/// let ok = Url::parse("http://192.168.1.1:5000/ctl").unwrap();
/// assert!(trusted_endpoint(&ok, responder, is_lan_address).is_ok());
/// let other = Url::parse("http://192.168.1.9/ctl").unwrap();
/// assert_eq!(trusted_endpoint(&other, responder, is_lan_address), Err(IgdError::Untrusted));
/// ```
pub fn trusted_endpoint(
    url: &Url,
    responder: IpAddr,
    accept: fn(IpAddr) -> bool,
) -> Result<Endpoint, IgdError> {
    if url.scheme() != "http" {
        return Err(IgdError::Untrusted);
    }
    let ip = match url.host() {
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
        _ => return Err(IgdError::Untrusted),
    };
    if ip != responder || !accept(ip) {
        return Err(IgdError::Untrusted);
    }
    let port = url.port_or_known_default().ok_or(IgdError::Untrusted)?;
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(Endpoint {
        addr: SocketAddr::new(ip, port),
        path,
    })
}

/// A parsed HTTP reply.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::soap::HttpReply;
/// let reply = HttpReply { status: 200, body: Vec::new() };
/// assert_eq!(reply.status, 200);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    /// The numeric status code.
    pub status: u16,
    /// The body, at most [`MAX_DOCUMENT_BYTES`] long.
    pub body: Vec<u8>,
}

/// Finds the end of the header block, returning the offset of the body.
fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at.saturating_add(4))
}

/// Parses the buffer as an HTTP reply. Returns `Ok(None)` while more bytes are needed and
/// `eof` is false.
fn parse_reply(buf: &[u8], eof: bool) -> Result<Option<HttpReply>, IgdError> {
    let Some(body_start) = head_end(buf) else {
        if buf.len() > MAX_HEAD_BYTES {
            return Err(IgdError::TooLarge);
        }
        return if eof {
            Err(IgdError::Malformed("reply ended inside the header"))
        } else {
            Ok(None)
        };
    };
    if body_start > MAX_HEAD_BYTES {
        return Err(IgdError::TooLarge);
    }
    let head = std::str::from_utf8(buf.get(..body_start).unwrap_or(&[]))
        .map_err(|_| IgdError::Malformed("header is not text"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(IgdError::Malformed("not an HTTP reply"));
    }
    let status: u16 = parts
        .next()
        .and_then(|code| code.parse().ok())
        .ok_or(IgdError::Malformed("bad status code"))?;
    let mut content_length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            return Err(IgdError::Malformed("chunked replies are not accepted"));
        }
        if name.trim().eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse()
                    .map_err(|_| IgdError::Malformed("bad content-length"))?,
            );
        }
    }
    if content_length.is_some_and(|len| len > MAX_DOCUMENT_BYTES) {
        return Err(IgdError::TooLarge);
    }
    let body = buf.get(body_start..).unwrap_or(&[]);
    if body.len() > MAX_DOCUMENT_BYTES {
        return Err(IgdError::TooLarge);
    }
    match content_length {
        Some(len) if body.len() >= len => Ok(Some(HttpReply {
            status,
            body: body.get(..len).unwrap_or(body).to_vec(),
        })),
        _ if eof => Ok(Some(HttpReply {
            status,
            body: body.to_vec(),
        })),
        _ => Ok(None),
    }
}

/// Sends one HTTP/1.0 request and reads the reply, bounded by [`MAX_DOCUMENT_BYTES`] and
/// `deadline`. HTTP/1.0 keeps routers from chunking the reply; a chunked reply is refused.
///
/// # Errors
///
/// [`IgdError::Timeout`], [`IgdError::Io`], [`IgdError::TooLarge`] or [`IgdError::Malformed`].
///
/// # Examples
///
/// ```no_run
/// use shadowcat::portmap::soap::{http_request, Endpoint, HTTP_DEADLINE};
/// # async fn demo() {
/// let endpoint = Endpoint { addr: "192.168.1.1:5000".parse().unwrap(), path: "/desc.xml".into() };
/// let reply = http_request(&endpoint, "GET", &[], b"", HTTP_DEADLINE).await;
/// # let _ = reply;
/// # }
/// ```
pub async fn http_request(
    endpoint: &Endpoint,
    method: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    deadline: Duration,
) -> Result<HttpReply, IgdError> {
    let exchange = async {
        let mut stream = TcpStream::connect(endpoint.addr)
            .await
            .map_err(|e| IgdError::Io(e.kind()))?;
        let mut head = format!(
            "{method} {} HTTP/1.0\r\nHost: {}\r\n",
            endpoint.path, endpoint.addr
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|e| IgdError::Io(e.kind()))?;
        stream
            .write_all(body)
            .await
            .map_err(|e| IgdError::Io(e.kind()))?;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|e| IgdError::Io(e.kind()))?;
            let eof = read == 0;
            buf.extend_from_slice(chunk.get(..read).unwrap_or(&[]));
            if let Some(reply) = parse_reply(&buf, eof)? {
                return Ok(reply);
            }
            if eof {
                return Err(IgdError::Malformed("reply ended early"));
            }
        }
    };
    tokio::time::timeout(deadline, exchange)
        .await
        .map_err(|_| IgdError::Timeout)?
}

/// Calls one SOAP action and returns the response body text. A non-200 reply carrying a UPnP
/// fault becomes [`IgdError::Fault`].
///
/// # Errors
///
/// The errors of [`http_request`], [`IgdError::Fault`] and [`IgdError::Status`].
///
/// # Examples
///
/// ```no_run
/// use shadowcat::portmap::soap::{soap_call, Endpoint};
/// # async fn demo() {
/// let endpoint = Endpoint { addr: "192.168.1.1:5000".parse().unwrap(), path: "/ctl".into() };
/// let reply = soap_call(&endpoint, "urn:schemas-upnp-org:service:WANIPConnection:1",
///     "GetExternalIPAddress", &[]).await;
/// # let _ = reply;
/// # }
/// ```
pub async fn soap_call(
    endpoint: &Endpoint,
    service_type: &str,
    action: &str,
    args: &[(&str, String)],
) -> Result<String, IgdError> {
    let envelope = soap_envelope(service_type, action, args);
    let action_header = format!("\"{service_type}#{action}\"");
    let reply = http_request(
        endpoint,
        "POST",
        &[
            ("Content-Type", "text/xml; charset=\"utf-8\""),
            ("SOAPAction", &action_header),
        ],
        envelope.as_bytes(),
        HTTP_DEADLINE,
    )
    .await?;
    let text =
        String::from_utf8(reply.body).map_err(|_| IgdError::Malformed("body is not text"))?;
    if reply.status == 200 {
        return Ok(text);
    }
    match fault_code(&text) {
        Ok(Some(code)) => Err(IgdError::Fault(code)),
        _ => Err(IgdError::Status(reply.status)),
    }
}

#[cfg(test)]
mod tests;
