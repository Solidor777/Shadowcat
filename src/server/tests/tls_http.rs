//! The HTTP surface over the real sniffing listener: per-scheme session cookies, logout removal,
//! HSTS, `wss://` and HTTP/2 WebSocket round trips, `ClientIp`, the published TLS status and the
//! admin fingerprint route, and plain-HTTP survival under every TLS degradation.

use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use shadowcat::auth::password::hash_password;
use shadowcat::auth::role::ServerRole;
use shadowcat::config::Config;
use shadowcat::tls::selfsigned::{generate, CertGenerator, Generated, SanSet, SanSources};
use shadowcat::tls::{TlsError, TlsRuntime};
use shadowcat_test_support::tls::{h2_connect, handshake_any, sha256_hex};
use shadowcat_test_support::{
    create_doc_op, drain_until_event, drain_until_type, intent_msg, spawn, spawn_tls,
    spawn_tls_with, spawn_with, spawn_with_runtime, test_doc_id, Harness,
};
use tokio_tungstenite::tungstenite::Message;

fn set_cookies(res: &reqwest::Response) -> Vec<String> {
    res.headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

async fn login(client: &reqwest::Client, base: &str) -> reqwest::Response {
    client
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap()
}

fn plain_client() -> reqwest::Client {
    reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn login_over_https_sets_a_host_prefixed_secure_cookie_and_over_http_a_plain_one() {
    let h = spawn_tls().await;
    let https = h.https_client();
    let res = login(&https, &format!("https://{}", h.addr)).await;
    assert_eq!(res.status(), 204);
    let cookies = set_cookies(&res);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    let c = &cookies[0];
    assert!(c.starts_with("__Host-sc_sid="), "{c}");
    for attr in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(c.contains(attr), "{attr} missing from {c}");
    }
    assert!(!c.to_ascii_lowercase().contains("domain="), "{c}");

    let plain = plain_client();
    let res = login(&plain, &format!("http://{}", h.addr)).await;
    assert_eq!(res.status(), 204);
    let c = &set_cookies(&res)[0];
    assert!(c.starts_with("sc_sid="), "{c}");
    assert!(!c.to_ascii_lowercase().contains("secure"), "{c}");
}

#[tokio::test]
async fn both_schemes_work_independently_with_separate_sessions() {
    let h = spawn_tls().await;
    let https = h.https_client();
    let plain = plain_client();
    login(&https, &format!("https://{}", h.addr)).await;
    login(&plain, &format!("http://{}", h.addr)).await;
    for (client, base) in [
        (&https, format!("https://{}", h.addr)),
        (&plain, format!("http://{}", h.addr)),
    ] {
        let me = client.get(format!("{base}/api/me")).send().await.unwrap();
        assert_eq!(me.status(), 200, "{base}");
    }
    // A cookie minted over HTTPS carries the other name: the plain-HTTP flavour does not accept
    // it (one session per scheme).
    let https_cookie = h
        .https_client()
        .post(format!("https://{}/api/login", h.addr))
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let raw = set_cookies(&https_cookie)[0]
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let over_http = reqwest::Client::new()
        .get(format!("http://{}/api/me", h.addr))
        .header("cookie", raw)
        .send()
        .await
        .unwrap();
    assert_eq!(over_http.status(), 401);
}

#[tokio::test]
async fn logout_over_https_emits_a_removal_cookie_that_is_secure_and_root_pathed() {
    let h = spawn_tls().await;
    let https = h.https_client();
    let base = format!("https://{}", h.addr);
    login(&https, &base).await;
    let out = https
        .post(format!("{base}/api/logout"))
        .send()
        .await
        .unwrap();
    assert!(out.status().is_success(), "{}", out.status());
    let cookies = set_cookies(&out);
    let removal = cookies
        .iter()
        .find(|c| c.starts_with("__Host-sc_sid="))
        .unwrap_or_else(|| panic!("no removal cookie in {cookies:?}"));
    // A `__Host-` Set-Cookie without Secure/Path=/ is ignored by browsers, so the removal cookie
    // would never delete anything: the normaliser must have added them.
    assert!(removal.contains("Secure"), "{removal}");
    assert!(removal.contains("Path=/"), "{removal}");
    assert!(
        removal.contains("Max-Age=0") || removal.contains("1970"),
        "{removal}"
    );
    // And the session is really gone.
    let me = https.get(format!("{base}/api/me")).send().await.unwrap();
    assert_eq!(me.status(), 401);
}

#[tokio::test]
async fn a_trusted_proxy_forwarding_https_gets_the_secure_cookie_and_an_untrusted_one_does_not() {
    // The proxy is the loopback peer; trusted_proxies lists it.
    let trusted = spawn_tls_with(|c| c.trusted_proxies = vec!["127.0.0.1".parse().unwrap()]).await;
    let res = plain_client()
        .post(format!("http://{}/api/login", trusted.addr))
        .header("x-forwarded-proto", "https")
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let c = &set_cookies(&res)[0];
    assert!(
        c.starts_with("__Host-sc_sid=") && c.contains("Secure"),
        "{c}"
    );

    // Untrusted peer: the header is ignored entirely.
    let untrusted = spawn_tls().await;
    let res = plain_client()
        .post(format!("http://{}/api/login", untrusted.addr))
        .header("x-forwarded-proto", "https")
        .json(&serde_json::json!({ "username": "u", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let c = &set_cookies(&res)[0];
    assert!(
        c.starts_with("sc_sid=") && !c.to_ascii_lowercase().contains("secure"),
        "{c}"
    );
}

// ---------------------------------------------------------------------------------------------
// HSTS
// ---------------------------------------------------------------------------------------------

/// A harness whose primary certificate is an operator PEM covering `game.example`.
async fn pem_harness(
    hsts: Option<u64>,
    trusted: &[&str],
) -> (Harness, Generated, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let g = generate(
        &SanSet {
            dns: vec!["game.example".into()],
            ips: vec![],
        },
        time::OffsetDateTime::now_utc(),
    )
    .unwrap();
    let cert = dir.path().join("c.pem");
    let key = dir.path().join("k.pem");
    std::fs::write(&cert, &g.cert_pem).unwrap();
    std::fs::write(&key, &g.key_pem).unwrap();
    let trusted: Vec<std::net::IpAddr> = trusted.iter().map(|s| s.parse().unwrap()).collect();
    let h = spawn_tls_with(move |c| {
        c.tls_cert = Some(cert.to_string_lossy().into_owned());
        c.tls_key = Some(key.to_string_lossy().into_owned());
        c.hsts_max_age_secs = hsts;
        c.trusted_proxies = trusted;
    })
    .await;
    (h, g, dir)
}

/// A client that trusts the operator certificate and resolves `game.example` to the harness.
fn named_client(h: &Harness, pem: &Generated) -> reqwest::Client {
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_der(pem.cert_der.as_ref()).unwrap())
        .resolve("game.example", h.socket_addr())
        .build()
        .unwrap()
}

#[tokio::test]
async fn hsts_matrix_default_none_opt_in_on_trusted_native_tls_only() {
    let hsts = "strict-transport-security";

    // Default: never.
    let (h, pem, _d) = pem_harness(None, &[]).await;
    let named = named_client(&h, &pem);
    let res = named
        .get(format!(
            "https://game.example:{}/health",
            h.socket_addr().port()
        ))
        .send()
        .await
        .unwrap();
    assert!(res.headers().get(hsts).is_none());

    // Opt-in: an SNI-covered native-TLS request (primary certificate) gets it, exactly max-age.
    let (h, pem, _d) = pem_harness(Some(600), &["127.0.0.1"]).await;
    let port = h.socket_addr().port();
    let named = named_client(&h, &pem);
    let res = named
        .get(format!("https://game.example:{port}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.headers().get(hsts).unwrap(), "max-age=600");

    // A native-TLS IP-literal connection (self-signed fallback) NEVER gets it, even though the
    // configured source is the operator certificate.
    let ip_client = h.https_client();
    let res = ip_client
        .get(format!("https://{}/health", h.addr))
        .send()
        .await
        .unwrap();
    assert!(
        res.headers().get(hsts).is_none(),
        "fallback must never receive HSTS"
    );

    // A plain request never does.
    let res = reqwest::get(format!("http://{}/health", h.addr))
        .await
        .unwrap();
    assert!(res.headers().get(hsts).is_none());

    // A trusted-proxy request forwarding https does (the client sees the proxy's certificate).
    let res = reqwest::Client::new()
        .get(format!("http://{}/health", h.addr))
        .header("x-forwarded-proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(res.headers().get(hsts).unwrap(), "max-age=600");

    // No includeSubDomains, no preload.
    let v = res
        .headers()
        .get(hsts)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(!v.contains("includeSubDomains") && !v.contains("preload"));
}

#[tokio::test]
async fn hsts_is_never_sent_on_a_self_signed_deployment_even_when_opted_in() {
    let h = spawn_tls_with(|c| c.hsts_max_age_secs = Some(600)).await;
    let res = h
        .https_client()
        .get(format!("https://{}/health", h.addr))
        .send()
        .await
        .unwrap();
    assert!(res.headers().get("strict-transport-security").is_none());
}

// ---------------------------------------------------------------------------------------------
// WebSockets
// ---------------------------------------------------------------------------------------------

async fn intent_round_trip<S>(ws: &mut S, world: uuid::Uuid, id: u128)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message>
        + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Debug,
{
    let _ = drain_until_type(ws, "welcome").await;
    ws.send(intent_msg(
        id as u64,
        serde_json::json!([create_doc_op(world, id, None, "scene")]),
    ))
    .await
    .unwrap();
    let ev = drain_until_event(ws).await;
    assert_eq!(ev["type"], "event");
    let _ = test_doc_id(id);
}

#[tokio::test]
async fn wss_and_ws_share_one_port_and_both_round_trip_an_intent() {
    let h = spawn_tls().await;
    // wss:// with the HTTPS-flavour cookie.
    let https = h.https_client();
    let res = login(&https, &format!("https://{}", h.addr)).await;
    let cookie = set_cookies(&res)[0].split(';').next().unwrap().to_string();
    let mut wss = h.connect_tls_as(&cookie).await;
    intent_round_trip(&mut wss, h.world, 101).await;
    // ws:// on the same port with the plain cookie (the harness cookie).
    let mut ws = h.connect().await;
    intent_round_trip(&mut ws, h.world, 102).await;
}

#[tokio::test]
async fn ws_over_http2_extended_connect_round_trips_with_compression_offered() {
    let h = spawn_tls().await;
    let https = h.https_client();
    let res = login(&https, &format!("https://{}", h.addr)).await;
    let cookie = set_cookies(&res)[0].split(';').next().unwrap().to_string();

    let (mut client, alpn) = h2_connect(h.socket_addr()).await.unwrap();
    assert_eq!(
        alpn.as_deref(),
        Some(&b"h2"[..]),
        "the server negotiates h2 when offered"
    );
    let (status, pipe) = client
        .websocket(&h.addr, &format!("/ws?world={}", h.world), &cookie)
        .await
        .unwrap();
    assert_eq!(status, 200);
    let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
        pipe,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    tokio::time::timeout(
        Duration::from_secs(20),
        intent_round_trip(&mut ws, h.world, 103),
    )
    .await
    .expect("h2 websocket round trip timed out");
}

#[tokio::test]
async fn alpn_prefers_h2_and_falls_back_to_http11() {
    let h = spawn_tls().await;
    let addr = h.socket_addr();
    let (_s, p) = handshake_any(addr, None, &["h2", "http/1.1"])
        .await
        .unwrap();
    assert_eq!(p.alpn.as_deref(), Some(&b"h2"[..]));
    let (_s, p) = handshake_any(addr, None, &["http/1.1"]).await.unwrap();
    assert_eq!(p.alpn.as_deref(), Some(&b"http/1.1"[..]));
    // The public status publishes exactly what is negotiated.
    let cfg: serde_json::Value = h
        .https_client()
        .get(format!("https://{}/api/config", h.addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["tls"]["alpn"], serde_json::json!(["h2", "http/1.1"]));
}

// ---------------------------------------------------------------------------------------------
// ClientIp and status
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn client_ip_resolves_under_every_listener_so_the_per_ip_throttle_fires() {
    // Identity budget high, per-IP budget 3: only a resolved client IP can trip the throttle. The
    // three modes are the TLS-off listener, the sniffing listener on a plain connection and the
    // sniffing listener on a TLS connection; each builds its service through `into_service`.
    for mode in ["tls-off", "sniff-plain", "sniff-tls"] {
        let tune = |c: &mut Config| {
            c.login_per_min_per_identity = Some(1000);
            c.login_per_min_per_ip = Some(3);
        };
        let h = if mode == "tls-off" {
            spawn_with(tune).await
        } else {
            spawn_tls_with(tune).await
        };
        let (client, base) = if mode == "sniff-tls" {
            (h.https_client(), format!("https://{}", h.addr))
        } else {
            (plain_client(), format!("http://{}", h.addr))
        };
        let mut statuses = Vec::new();
        for i in 0..6 {
            let res = client
                .post(format!("{base}/api/login"))
                .json(&serde_json::json!({ "username": format!("nobody-{mode}-{i}"), "password": "x" }))
                .send()
                .await
                .unwrap();
            statuses.push(res.status().as_u16());
        }
        assert!(
            statuses.contains(&429),
            "per-IP throttle never fired under {mode}: {statuses:?}"
        );
    }
}

#[tokio::test]
async fn api_config_publishes_a_secret_free_tls_block() {
    let h = spawn_tls().await;
    let cfg: serde_json::Value = plain_client()
        .get(format!("http://{}/api/config", h.addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tls = &cfg["tls"];
    assert_eq!(tls["enabled"], true);
    assert_eq!(tls["source"], "self-signed");
    assert!(tls["https_origins"].as_array().unwrap().is_empty());
    let text = tls.to_string();
    assert!(
        !text.contains("fingerprint") && !text.contains("sha256"),
        "{text}"
    );

    let off = spawn().await;
    let cfg: serde_json::Value = plain_client()
        .get(format!("http://{}/api/config", off.addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["tls"]["enabled"], false);
}

#[tokio::test]
async fn admin_route_publishes_the_fingerprint_of_the_certificate_a_client_reads() {
    let h = spawn_tls().await;
    // A real admin, a plain user (the harness account is a GM of its world but only a `User` at
    // the server tier), and an anonymous caller.
    let hash = hash_password("pw").unwrap();
    h.repo
        .create_user("root", Some(&hash), ServerRole::Admin, 0)
        .await
        .unwrap();
    let base = format!("https://{}", h.addr);

    let anon = h
        .https_client()
        .get(format!("{base}/api/admin/tls"))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);

    let user = h.https_client();
    login(&user, &base).await;
    let denied = user
        .get(format!("{base}/api/admin/tls"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403, "a world GM is not a server admin");

    let admin = h.https_client();
    let res = admin
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "username": "root", "password": "pw" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    let body: serde_json::Value = admin
        .get(format!("{base}/api/admin/tls"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let (_s, presented) = handshake_any(h.socket_addr(), None, &["http/1.1"])
        .await
        .unwrap();
    let read = sha256_hex(&presented.leaf);
    assert_eq!(body["fallback_fingerprint"], read.as_str());
    assert_eq!(
        body["fingerprint"],
        read.as_str(),
        "self-signed source: same certificate"
    );
    assert!(body["not_after"].as_i64().unwrap() > 0);
    assert!(body["degraded_from"].is_null());
    assert_eq!(body["enabled"], true);
}

// ---------------------------------------------------------------------------------------------
// Plain HTTP survives every degradation
// ---------------------------------------------------------------------------------------------

async fn assert_plain_http_serves(h: &Harness) {
    let plain = plain_client();
    let health = plain
        .get(format!("http://{}/health", h.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let res = login(&plain, &format!("http://{}", h.addr)).await;
    assert_eq!(res.status(), 204);
}

struct FailingGenerator;

impl CertGenerator for FailingGenerator {
    fn generate(&self, _s: &SanSet, _n: time::OffsetDateTime) -> Result<Generated, TlsError> {
        Err(TlsError::Generate("forced generation failure".into()))
    }
}

#[tokio::test]
async fn plain_http_health_and_login_survive_each_tls_degradation() {
    // Expired operator PEM.
    let dir = tempfile::tempdir().unwrap();
    let old = generate(
        &SanSet::default(),
        time::OffsetDateTime::now_utc() - time::Duration::days(500),
    )
    .unwrap();
    let (c, k) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
    std::fs::write(&c, &old.cert_pem).unwrap();
    std::fs::write(&k, &old.key_pem).unwrap();
    let h = spawn_tls_with(|cfg| {
        cfg.tls_cert = Some(c.to_string_lossy().into_owned());
        cfg.tls_key = Some(k.to_string_lossy().into_owned());
    })
    .await;
    assert_plain_http_serves(&h).await;
    assert_eq!(
        h.tls.status().source,
        Some(shadowcat::tls::TlsSource::SelfSigned)
    );

    // Garbage PEM.
    let (gc, gk) = (dir.path().join("gc.pem"), dir.path().join("gk.pem"));
    std::fs::write(&gc, "junk").unwrap();
    std::fs::write(&gk, "junk").unwrap();
    let h = spawn_tls_with(|cfg| {
        cfg.tls_cert = Some(gc.to_string_lossy().into_owned());
        cfg.tls_key = Some(gk.to_string_lossy().into_owned());
    })
    .await;
    assert_plain_http_serves(&h).await;

    // Unwritable tls_dir.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let h = spawn_tls_with(|cfg| {
        cfg.tls_dir = Some(blocker.join("tls").to_string_lossy().into_owned())
    })
    .await;
    assert_plain_http_serves(&h).await;
    assert!(h.tls.status().enabled);

    // ACME configured but unreachable / unavailable.
    let acme_dir = dir.path().join("acme-tls");
    let h = spawn_tls_with(|cfg| {
        cfg.acme_domains = vec!["game.example".into()];
        cfg.acme_terms_agreed = true;
        cfg.acme_ca_root = Some("no-such-root.pem".into());
        cfg.tls_dir = Some(acme_dir.to_string_lossy().into_owned());
    })
    .await;
    assert_plain_http_serves(&h).await;
    assert_eq!(
        h.tls.admin_status().degraded_from,
        Some(shadowcat::tls::TlsSource::Acme)
    );

    // Forced generation failure: TLS off for the process, plain HTTP unaffected, and a TLS peer
    // is dropped rather than served.
    let h = spawn_with_runtime(
        |_| {},
        |cfg| {
            TlsRuntime::start_with(
                cfg,
                Arc::new(FailingGenerator),
                SanSources::default(),
                time::OffsetDateTime::now_utc(),
            )
        },
    )
    .await;
    assert_plain_http_serves(&h).await;
    assert!(!h.tls.status().enabled);
    assert!(handshake_any(h.socket_addr(), None, &["http/1.1"])
        .await
        .is_err());
}
