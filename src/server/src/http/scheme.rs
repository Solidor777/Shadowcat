//! The request scheme, decided in exactly one place.
//!
//! [`classify`] is the ONE decider. [`request_scheme`] (session cookie flavour and any absolute-URL
//! builder) and [`hsts_max_age`] (the HSTS gate) both read it, so the three cannot disagree on an
//! input. A request is HTTPS when
//! 1. this process terminated TLS for its connection (`ConnMeta.tls`), or
//! 2. its immediate peer is a configured trusted proxy AND the rightmost comma-separated value of
//!    the last `X-Forwarded-Proto` header line is `https` (ASCII case-insensitive): the rightmost
//!    value is the assertion of the nearest trusted proxy, and a multi-hop chain whose inner proxy
//!    overwrote it to `http` yields plain HTTP, which fails toward a non-`Secure` cookie that still
//!    works on an HTTPS page. RFC 7239 `Forwarded` is ignored, and an untrusted peer's header is
//!    ignored entirely.
//!
//! Anything else (including a request with no `ConnectInfo`, as under the axum-test mock
//! transport) is plain HTTP.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use axum::extract::ConnectInfo;
use axum::http::{Extensions, HeaderMap};
use axum::Router;

use crate::config::Config;
use crate::tls::{ConnMeta, Selected};

/// Which scheme the CLIENT used to reach the server.
///
/// # Examples
///
/// ```
/// use shadowcat::http::scheme::RequestScheme;
///
/// assert_ne!(RequestScheme::Http, RequestScheme::Https);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestScheme {
    /// Plain HTTP.
    Http,
    /// HTTPS, terminated by this process or by a trusted proxy.
    Https,
}

/// How a request reached the process, the finest distinction any scheme-dependent decision needs.
///
/// # Examples
///
/// ```
/// use shadowcat::http::scheme::{classify, Reach};
///
/// let r = classify(&axum::http::Extensions::new(), &axum::http::HeaderMap::new(), &shadowcat::config::Config::default());
/// assert_eq!(r, Reach::Plain);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Plain HTTP directly to this process.
    Plain,
    /// TLS terminated by this process; carries which certificate configuration served it.
    Native(Selected),
    /// HTTPS terminated by a trusted proxy in front of this process.
    Proxy,
}

/// The single decider: classifies a request from its connection metadata and headers.
///
/// # Examples
///
/// ```
/// use axum::extract::ConnectInfo;
/// use axum::http::{Extensions, HeaderMap};
/// use shadowcat::http::scheme::{classify, Reach};
/// use shadowcat::tls::{ConnMeta, Selected};
///
/// let mut ext = Extensions::new();
/// ext.insert(ConnectInfo(ConnMeta::tls("127.0.0.1:1".parse().unwrap(), Selected::Fallback)));
/// let reach = classify(&ext, &HeaderMap::new(), &shadowcat::config::Config::default());
/// assert_eq!(reach, Reach::Native(Selected::Fallback));
/// ```
pub fn classify(extensions: &Extensions, headers: &HeaderMap, config: &Config) -> Reach {
    let Some(ConnectInfo(meta)) = extensions.get::<ConnectInfo<ConnMeta>>() else {
        return Reach::Plain;
    };
    if meta.tls {
        return match meta.cert {
            Some(selected) => Reach::Native(selected),
            None => Reach::Plain,
        };
    }
    if config.is_trusted_proxy(meta.peer.ip()) && forwarded_proto_is_https(headers) {
        return Reach::Proxy;
    }
    Reach::Plain
}

/// Whether the rightmost value of the last `X-Forwarded-Proto` line is `https`.
fn forwarded_proto_is_https(headers: &HeaderMap) -> bool {
    headers
        .get_all("x-forwarded-proto")
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"))
}

/// The scheme the client used, derived from [`classify`].
///
/// # Examples
///
/// ```
/// use axum::http::{Extensions, HeaderMap};
/// use shadowcat::http::scheme::{request_scheme, RequestScheme};
///
/// let scheme = request_scheme(&Extensions::new(), &HeaderMap::new(), &shadowcat::config::Config::default());
/// assert_eq!(scheme, RequestScheme::Http);
/// ```
pub fn request_scheme(
    extensions: &Extensions,
    headers: &HeaderMap,
    config: &Config,
) -> RequestScheme {
    match classify(extensions, headers, config) {
        Reach::Plain => RequestScheme::Http,
        Reach::Native(_) | Reach::Proxy => RequestScheme::Https,
    }
}

/// The `Strict-Transport-Security` max-age to send for this request, or `None`.
///
/// HSTS is per host, so a single header converts every future `http://host:*` in that browser to
/// HTTPS and ends plain-HTTP play there; hence it is opt-in (`Config.hsts_max_age_secs`) and sent
/// only when the CLIENT-FACING scheme is HTTPS AND the certificate the client saw is one it
/// trusts: a connection served with the self-signed fallback (IP-literal access, no SNI, an
/// uncovered name, an expired primary) never gets it, whatever `TlsStatus.source` says. A
/// trusted-proxy request qualifies because the client sees the proxy's certificate, not ours.
///
/// # Examples
///
/// ```
/// use axum::extract::ConnectInfo;
/// use axum::http::{Extensions, HeaderMap};
/// use shadowcat::config::Config;
/// use shadowcat::http::scheme::hsts_max_age;
/// use shadowcat::tls::{ConnMeta, Selected};
///
/// let cfg = Config { hsts_max_age_secs: Some(600), ..Default::default() };
/// let mut ext = Extensions::new();
/// ext.insert(ConnectInfo(ConnMeta::tls("127.0.0.1:1".parse().unwrap(), Selected::Primary)));
/// assert_eq!(hsts_max_age(&ext, &HeaderMap::new(), &cfg), Some(600));
/// let mut fallback = Extensions::new();
/// fallback.insert(ConnectInfo(ConnMeta::tls("127.0.0.1:1".parse().unwrap(), Selected::Fallback)));
/// assert_eq!(hsts_max_age(&fallback, &HeaderMap::new(), &cfg), None);
/// ```
pub fn hsts_max_age(extensions: &Extensions, headers: &HeaderMap, config: &Config) -> Option<u64> {
    let max_age = config.hsts_max_age_secs?;
    match classify(extensions, headers, config) {
        Reach::Proxy | Reach::Native(Selected::Primary) => Some(max_age),
        Reach::Plain | Reach::Native(Selected::Fallback | Selected::Challenge) => None,
    }
}

/// The ONLY place `into_make_service_with_connect_info::<ConnMeta>()` is spelled: the make-service
/// for transports served by `axum::serve` (the real-socket test harnesses over a plain
/// `TcpListener` and the axum-test mock). The server itself serves through
/// [`crate::http::conn_serve::serve_router`], which inserts the same `ConnectInfo<ConnMeta>` per
/// connection. Those two are the only insertion points, so `ConnectInfo<ConnMeta>` (and with it
/// `ClientIp`, the scheme decider and the HSTS gate) has one shape regardless of listener type.
///
/// # Examples
///
/// ```no_run
/// # #[tokio::main] async fn main() {
/// let app = axum::Router::new();
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
/// axum::serve(listener, shadowcat::http::scheme::into_service(app)).await.unwrap();
/// # }
/// ```
pub fn into_service(app: Router) -> IntoMakeServiceWithConnectInfo<Router, ConnMeta> {
    app.into_make_service_with_connect_info::<ConnMeta>()
}

#[cfg(test)]
mod tests;
