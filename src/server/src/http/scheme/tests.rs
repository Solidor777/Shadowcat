use super::*;
use crate::auth::password::hash_password;
use crate::auth::role::ServerRole;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

/// One input row of the decision table.
struct Row {
    /// Human-readable name for failure messages.
    name: &'static str,
    /// Connection metadata, `None` for no `ConnectInfo` at all.
    meta: Option<ConnMeta>,
    /// `X-Forwarded-Proto` header lines, in order.
    xfp: &'static [&'static str],
    /// The decision every reader must agree on.
    expected: RequestScheme,
    /// Whether HSTS may be sent (given an opt-in).
    hsts: bool,
}

fn peer(ip: &str) -> std::net::SocketAddr {
    format!("{ip}:5000").parse().unwrap()
}

fn rows() -> Vec<Row> {
    let proxy = || Some(ConnMeta::plain(peer("10.0.0.2")));
    let plain = || Some(ConnMeta::plain(peer("192.0.2.5")));
    let tls = |sel| Some(ConnMeta::tls(peer("192.0.2.5"), sel));
    vec![
        Row {
            name: "native tls primary",
            meta: tls(Selected::Primary),
            xfp: &[],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "native tls fallback",
            meta: tls(Selected::Fallback),
            xfp: &[],
            expected: RequestScheme::Https,
            hsts: false,
        },
        Row {
            name: "plain, no header",
            meta: plain(),
            xfp: &[],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "trusted proxy https",
            meta: proxy(),
            xfp: &["https"],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "trusted proxy HTTPS",
            meta: proxy(),
            xfp: &["HTTPS"],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "trusted proxy http",
            meta: proxy(),
            xfp: &["http"],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "trusted proxy absent",
            meta: proxy(),
            xfp: &[],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "trusted proxy http, https",
            meta: proxy(),
            xfp: &["http, https"],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "trusted proxy https, http",
            meta: proxy(),
            xfp: &["https, http"],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "two lines https then http",
            meta: proxy(),
            xfp: &["https", "http"],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "two lines http then https",
            meta: proxy(),
            xfp: &["http", "https"],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "trusted proxy https, peer spelled as an IPv4-mapped IPv6 address",
            meta: Some(ConnMeta::plain(std::net::SocketAddr::new(
                "::ffff:10.0.0.2".parse().unwrap(),
                5000,
            ))),
            xfp: &["https"],
            expected: RequestScheme::Https,
            hsts: true,
        },
        Row {
            name: "untrusted peer https",
            meta: plain(),
            xfp: &["https"],
            expected: RequestScheme::Http,
            hsts: false,
        },
        Row {
            name: "no ConnectInfo with header",
            meta: None,
            xfp: &["https"],
            expected: RequestScheme::Http,
            hsts: false,
        },
    ]
}

fn config() -> Config {
    Config {
        trusted_proxies: vec!["10.0.0.2".parse().unwrap()],
        hsts_max_age_secs: Some(600),
        ..Config::default()
    }
}

fn parts_for(row: &Row) -> (axum::http::Extensions, HeaderMap) {
    let mut ext = axum::http::Extensions::new();
    if let Some(m) = &row.meta {
        ext.insert(ConnectInfo(m.clone()));
    }
    let mut headers = HeaderMap::new();
    for line in row.xfp {
        headers.append("x-forwarded-proto", line.parse().unwrap());
    }
    (ext, headers)
}

#[test]
fn request_scheme_decision_table() {
    let cfg = config();
    for row in rows() {
        let (ext, headers) = parts_for(&row);
        assert_eq!(
            request_scheme(&ext, &headers, &cfg),
            row.expected,
            "{}",
            row.name
        );
    }
}

#[test]
fn a_loopback_peer_is_not_believed_about_the_scheme_unless_it_is_listed() {
    // Loopback peers are exempt from the per-peer connection caps, which is not trust.
    let cfg = Config {
        hsts_max_age_secs: Some(600),
        ..Config::default()
    };
    for loopback in ["127.0.0.1", "::1"] {
        let host = if loopback.contains(':') {
            format!("[{loopback}]")
        } else {
            loopback.to_string()
        };
        let row = Row {
            name: "loopback peer asserting https",
            meta: Some(ConnMeta::plain(peer(&host))),
            xfp: &["https"],
            expected: RequestScheme::Http,
            hsts: false,
        };
        let (ext, headers) = parts_for(&row);
        assert_eq!(
            request_scheme(&ext, &headers, &cfg),
            row.expected,
            "{loopback}"
        );
        assert!(hsts_max_age(&ext, &headers, &cfg).is_none(), "{loopback}");
    }
}

#[test]
fn hsts_gate_agrees_with_the_decider_over_the_same_table() {
    let cfg = config();
    for row in rows() {
        let (ext, headers) = parts_for(&row);
        let sent = hsts_max_age(&ext, &headers, &cfg).is_some();
        assert_eq!(sent, row.hsts, "{}", row.name);
        // HSTS is only ever a subset of HTTPS requests.
        if sent {
            assert_eq!(
                request_scheme(&ext, &headers, &cfg),
                RequestScheme::Https,
                "{}",
                row.name
            );
        }
    }
    // Without an opt-in nothing is ever sent.
    let off = Config {
        hsts_max_age_secs: None,
        ..config()
    };
    for row in rows() {
        let (ext, headers) = parts_for(&row);
        assert!(hsts_max_age(&ext, &headers, &off).is_none(), "{}", row.name);
    }
}

/// Serves `POST /api/login` through the real router for `row` and returns the `Set-Cookie`
/// header plus the HSTS header.
async fn login_through_router(row: &Row) -> (String, Option<String>) {
    let mut state = crate::http::tests::initialized_state().await;
    state.config = std::sync::Arc::new(config());
    state
        .repo
        .create_user(
            "u",
            Some(&hash_password("pw").unwrap()),
            ServerRole::User,
            0,
        )
        .await
        .unwrap();
    let app = crate::http::router(state).await;
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"username":"u","password":"pw"}"#))
        .unwrap();
    let (ext, headers) = parts_for(row);
    *req.extensions_mut() = ext;
    req.headers_mut().extend(headers);
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT, "{}", row.name);
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let hsts = res
        .headers()
        .get(header::STRICT_TRANSPORT_SECURITY)
        .map(|v| v.to_str().unwrap().to_string());
    (cookie, hsts)
}

/// Anti-drift: the cookie flavour (name + `Secure`), the HSTS header and `request_scheme` are
/// three readers of one decision; over the same table they may never disagree.
#[tokio::test]
async fn cookie_flavour_and_hsts_header_follow_request_scheme_over_the_same_table() {
    for row in rows() {
        let (cookie, hsts) = login_through_router(&row).await;
        match row.expected {
            RequestScheme::Https => {
                assert!(
                    cookie.starts_with("__Host-sc_sid="),
                    "{}: {cookie}",
                    row.name
                );
                assert!(cookie.contains("Secure"), "{}: {cookie}", row.name);
            }
            RequestScheme::Http => {
                assert!(cookie.starts_with("sc_sid="), "{}: {cookie}", row.name);
                assert!(
                    !cookie.to_ascii_lowercase().contains("secure"),
                    "{}: {cookie}",
                    row.name
                );
            }
        }
        assert_eq!(hsts.is_some(), row.hsts, "{}", row.name);
        if let Some(v) = hsts {
            assert_eq!(v, "max-age=600", "{}", row.name);
        }
    }
}

#[test]
fn a_native_tls_connection_without_a_recorded_selection_is_treated_as_plain() {
    let mut ext = axum::http::Extensions::new();
    ext.insert(ConnectInfo(ConnMeta {
        peer: peer("192.0.2.5"),
        tls: true,
        cert: None,
        cap_exempt: false,
    }));
    assert_eq!(
        classify(&ext, &HeaderMap::new(), &Config::default()),
        Reach::Plain
    );
}

/// A non-prefixed cookie is left alone by the normaliser; a `__Host-` one is forced to the
/// prefix's requirements, dropping any `Domain`.
#[test]
fn the_normaliser_only_touches_host_prefixed_cookies() {
    use crate::auth::session::normalise_host_cookies;
    let mut h = HeaderMap::new();
    h.append(header::SET_COOKIE, "other=1; Path=/x".parse().unwrap());
    h.append(
        header::SET_COOKIE,
        "__Host-sc_sid=abc; Domain=evil.example; Path=/x"
            .parse()
            .unwrap(),
    );
    normalise_host_cookies(&mut h);
    let all: Vec<String> = h
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(all[0], "other=1; Path=/x");
    let host = &all[1];
    assert!(
        host.contains("Secure") && host.contains("HttpOnly") && host.contains("SameSite=Lax"),
        "{host}"
    );
    assert!(
        host.contains("Path=/") && !host.contains("Path=/x"),
        "{host}"
    );
    assert!(!host.to_ascii_lowercase().contains("domain"), "{host}");
}
