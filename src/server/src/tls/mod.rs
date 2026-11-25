//! Built-in HTTPS: one port answers both plain HTTP and TLS, chosen per connection by the first
//! byte ([`listener::SniffListener`]).
//!
//! Certificates come from an operator PEM ([`pem`]), a persisted self-signed leaf ([`selfsigned`])
//! or an in-memory self-signed leaf, chosen by the degradation ladder in [`TlsRuntime::start`].
//!
//! INVARIANT: A TLS-side failure of any kind (certificate, key file, handshake) never stops plain
//! HTTP.
//!
//! INVARIANT: Every rustls configuration names its crypto provider explicitly through
//! [`ring_provider`]; the process never relies on rustls' feature-based auto-selection.
//!
//! INVARIANT: The certificate a connection receives, its [`ConnMeta::cert`] and the HSTS decision
//! all derive from ONE [`resolver::Snapshot::select`] call (see [`resolver`]).
//!
//! INVARIANT: Two self-signed certificates exist. A peer on a private, link-local or
//! unique-local address that is not a loopback peer or a listed trusted proxy receives the one that
//! lists the machine's interface addresses; every other peer (including every loopback and
//! trusted-proxy peer, which fronts players the server cannot see) receives one that lists none (only `localhost`, loopback, `tls_hostnames` and the router's
//! external address when known). The choice is made once, from the accepted socket's peer address,
//! by [`resolver::PeerScope::of`], so a public player cannot read the host's LAN addresses from the
//! handshake.
//!
//! ACME seam: [`resolver::Snapshot`] carries the TLS-ALPN-01 challenge map and the
//! primary-certificate slot, and [`resolver::CertResolver::update`] swaps in a derived snapshot.
//! ACME status is reported through [`TlsSource::Acme`] and [`TlsRuntime`]'s `degraded_from` field.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod keyfile;
pub mod listener;
pub mod pem;
pub mod resolver;
pub mod selfsigned;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use serde::Serialize;
use time::OffsetDateTime;
use ts_rs::TS;

use crate::config::{AcmeChallenge, Config, TlsMode};
use crate::trash::Trash;

use self::pem::{PemPaths, PemStamp};
use self::resolver::{CertResolver, LoadedCert, Snapshot};
use self::selfsigned::{CertGenerator, RcgenGenerator, SanSources};

/// The ALPN protocols the server negotiates on serving connections, in preference order. Published
/// verbatim as `TlsStatus.alpn` so clients (and the browser test suite) read the server's truth
/// instead of keeping a second copy.
///
/// # Examples
///
/// ```
/// assert!(shadowcat::tls::ALPN_PROTOCOLS.contains(&"http/1.1"));
/// ```
pub const ALPN_PROTOCOLS: [&str; 2] = ["h2", "http/1.1"];

/// Errors of the TLS subsystem. Rendered into `TlsStatus`-adjacent diagnostics and logs.
///
/// # Examples
///
/// ```
/// let e = shadowcat::tls::TlsError::Cert("bad".into());
/// assert!(e.to_string().contains("bad"));
/// ```
#[derive(Debug, Clone, thiserror::Error)]
pub enum TlsError {
    /// A certificate, chain or key problem.
    #[error("{0}")]
    Cert(String),
    /// A PEM file could not be read or parsed.
    #[error("{0}")]
    Pem(String),
    /// A certificate outside its validity window.
    #[error("{0}")]
    Expired(String),
    /// A filesystem operation failed.
    #[error("{0}")]
    Io(String),
    /// Certificate generation failed (a genuine key-generation or signing fault).
    #[error("{0}")]
    Generate(String),
}

/// The one place the crypto provider is named: ring. Every rustls configuration in the crate is
/// built through this, so nothing depends on which rustls crate features happen to be enabled in
/// a given build graph.
///
/// # Examples
///
/// ```
/// let provider = shadowcat::tls::ring_provider();
/// assert!(!provider.cipher_suites.is_empty());
/// ```
pub fn ring_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Installs ring as the process-default provider (ignoring "already installed") so third-party
/// rustls users in the process cannot hit rustls' ambiguous-provider panic.
///
/// # Examples
///
/// ```
/// shadowcat::tls::install_default_provider();
/// shadowcat::tls::install_default_provider(); // idempotent
/// ```
pub fn install_default_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Which certificate configuration a TLS connection was served with. Recorded once per
/// connection from the single selection call.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::Selected;
///
/// assert_ne!(Selected::Primary, Selected::Fallback);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    /// A TLS-ALPN-01 validation certificate (the connection is closed after the handshake).
    Challenge,
    /// The CA-trusted certificate (operator PEM or ACME-issued).
    Primary,
    /// The self-signed certificate.
    Fallback,
}

/// Per-connection facts the listener hands to axum as `ConnectInfo`.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::{ConnMeta, Selected};
///
/// let plain = ConnMeta::plain("127.0.0.1:1".parse().unwrap());
/// assert!(!plain.tls && plain.cert.is_none());
/// let tls = ConnMeta::tls("127.0.0.1:2".parse().unwrap(), Selected::Fallback);
/// assert!(tls.tls && tls.cert == Some(Selected::Fallback));
/// ```
#[derive(Debug, Clone)]
pub struct ConnMeta {
    /// The TCP peer address, canonicalised (IPv4-mapped IPv6 reduced to IPv4).
    pub peer: SocketAddr,
    /// Whether the connection is TLS-terminated by this process.
    pub tls: bool,
    /// The selection outcome for a TLS connection; `None` for plain HTTP.
    pub cert: Option<Selected>,
    /// Whether the peer is exempt from the per-peer connection caps (`Config::is_cap_exempt`: a
    /// trusted proxy or a loopback peer; every player behind one shares its address). Set by the
    /// listener that admitted the connection; `false` from the constructors.
    pub cap_exempt: bool,
}

/// `peer` with an IPv4-mapped IPv6 address reduced to its IPv4 form, so every consumer of
/// [`ConnMeta::peer`] (trusted-proxy match, throttle keys, the client-IP resolver) sees one
/// address per host regardless of the socket's address family.
fn canonical_peer(mut peer: SocketAddr) -> SocketAddr {
    peer.set_ip(peer.ip().to_canonical());
    peer
}

impl ConnMeta {
    /// Metadata of a plain-HTTP connection.
    ///
    /// # Examples
    ///
    /// ```
    /// let m = shadowcat::tls::ConnMeta::plain("192.0.2.1:9".parse().unwrap());
    /// assert!(!m.tls);
    /// ```
    pub fn plain(peer: SocketAddr) -> Self {
        Self {
            peer: canonical_peer(peer),
            tls: false,
            cert: None,
            cap_exempt: false,
        }
    }

    /// Metadata of a TLS connection served with `selected`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::{ConnMeta, Selected};
    ///
    /// let m = ConnMeta::tls("192.0.2.1:9".parse().unwrap(), Selected::Primary);
    /// assert_eq!(m.cert, Some(Selected::Primary));
    /// ```
    pub fn tls(peer: SocketAddr, selected: Selected) -> Self {
        Self {
            peer: canonical_peer(peer),
            tls: true,
            cert: Some(selected),
            cap_exempt: false,
        }
    }
}

impl Connected<IncomingStream<'_, tokio::net::TcpListener>> for ConnMeta {
    fn connect_info(stream: IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        ConnMeta::plain(*stream.remote_addr())
    }
}

/// Where the served certificate comes from.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::TlsSource;
///
/// assert_eq!(serde_json::to_string(&TlsSource::SelfSigned).unwrap(), "\"self-signed\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../types/generated/")]
pub enum TlsSource {
    /// The persisted (or in-memory) self-signed certificate.
    SelfSigned,
    /// The operator-supplied PEM certificate.
    Operator,
    /// An ACME-issued certificate.
    Acme,
}

/// The public `tls` block of `GET /api/config`. Carries nothing secret: no fingerprint, no
/// paths, no error text.
///
/// # Examples
///
/// ```
/// let rt = shadowcat::tls::TlsRuntime::disabled();
/// let status = rt.status();
/// assert!(!status.enabled);
/// assert_eq!(status.alpn, vec!["h2".to_string(), "http/1.1".to_string()]);
/// ```
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct TlsStatus {
    /// Whether this process answers TLS.
    pub enabled: bool,
    /// Where the certificate a NAMED connection receives comes from; `None` when TLS is off.
    pub source: Option<TlsSource>,
    /// Explicitly configured `https://` origins players can use (never the machine hostname).
    pub https_origins: Vec<String>,
    /// The ALPN protocols the server negotiates.
    pub alpn: Vec<String>,
}

/// The admin-only view of the TLS subsystem: everything in [`TlsStatus`] plus the diagnostics that
/// must not reach players.
///
/// # Examples
///
/// ```
/// let rt = shadowcat::tls::TlsRuntime::disabled();
/// assert!(rt.admin_status().fingerprint.is_none());
/// ```
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct TlsAdminStatus {
    /// The public block.
    #[serde(flatten)]
    pub public: TlsStatus,
    /// SHA-256 (lowercase hex) of the leaf a NAMED connection receives (`source`'s certificate).
    pub fingerprint: Option<String>,
    /// SHA-256 (lowercase hex) of the self-signed leaf that IP-literal and SNI-less
    /// connections receive.
    pub fallback_fingerprint: Option<String>,
    /// `not_after` of the `fingerprint` certificate, unix seconds.
    #[ts(type = "number | null")]
    pub not_after: Option<i64>,
    /// The configured source the process fell back from, when a configured source failed.
    pub degraded_from: Option<TlsSource>,
    /// The most recent TLS-side error, when any.
    pub last_error: Option<String>,
}

/// Mutable runtime bookkeeping behind [`TlsRuntime`].
#[derive(Debug, Default)]
struct State {
    /// Source of the CA-trusted primary certificate currently installed, if any.
    primary_source: Option<TlsSource>,
    /// Configured source that failed and was replaced by self-signed.
    degraded_from: Option<TlsSource>,
    /// The most recent TLS-side error.
    last_error: Option<String>,
    /// Modification stamps of the operator PEM pair at the last load attempt.
    pem_stamp: PemStamp,
    /// Unix seconds of the last self-signed expiry check.
    last_selfsigned_check: i64,
}

/// Immutable inputs the runtime keeps for maintenance and status.
struct Settings {
    /// Directory holding the persisted self-signed material.
    tls_dir: Option<PathBuf>,
    /// Configured extra certificate hostnames.
    tls_hostnames: Vec<String>,
    /// Operator PEM locations.
    pem: Option<PemPaths>,
    /// Origins published regardless of certificate content.
    static_origins: Vec<String>,
    /// Whether `static_origins` is the operator's `public_url`, which is published alone.
    has_public_url: bool,
    /// Bind port, used to build published origins.
    bind_port: u16,
    /// Local facts the SAN list derives from.
    sources: SanSources,
    /// Certificate generator.
    generator: Arc<dyn CertGenerator>,
    /// Where a key temporary file left by a failed write goes.
    trash: Trash,
}

/// The TLS subsystem's runtime handle: the swap cell the listener reads and the status the HTTP
/// surface publishes.
///
/// # Examples
///
/// ```
/// let rt = shadowcat::tls::TlsRuntime::start(&shadowcat::config::Config {
///     db: "sqlite::memory:".into(),
///     ..Default::default()
/// });
/// assert!(rt.status().enabled);
/// assert!(rt.resolver().is_some());
/// ```
pub struct TlsRuntime {
    /// The swap cell; `None` when TLS is off for this process.
    resolver: Option<Arc<CertResolver>>,
    /// Mutable bookkeeping.
    state: Mutex<State>,
    /// Immutable inputs.
    settings: Settings,
    /// Where the router's public address is read from; `None` until
    /// [`TlsRuntime::set_external_source`] is called.
    external: Mutex<Option<ExternalSource>>,
}

/// Reads the router's public address, `None` while it is not known. The port-mapping runtime owns
/// the fact; the certificate shown to public peers lists whatever this returns.
pub type ExternalSource = Arc<dyn Fn() -> Option<IpAddr> + Send + Sync>;

/// How many seconds between self-signed expiry checks.
const SELFSIGNED_CHECK_SECS: i64 = 24 * 60 * 60;

/// Message reported while ACME issuance is unavailable in this build.
const ACME_UNAVAILABLE: &str =
    "ACME certificate issuance is not available in this build; serving the self-signed certificate";

impl TlsRuntime {
    /// A runtime with TLS off (the listener drops TLS-looking peers).
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// assert!(rt.resolver().is_none() && !rt.status().enabled);
    /// ```
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self::off(&Config::default(), None))
    }

    /// Builds an off-state runtime, remembering `error` for the admin status.
    fn off(config: &Config, error: Option<String>) -> Self {
        Self {
            resolver: None,
            external: Mutex::new(None),
            state: Mutex::new(State {
                last_error: error,
                ..State::default()
            }),
            settings: Settings::from_config(
                config,
                SanSources::default(),
                Arc::new(RcgenGenerator),
            ),
        }
    }

    /// Runs the degradation ladder for `config` with the real clock, host facts and generator.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::{Config, TlsMode};
    ///
    /// let off = shadowcat::tls::TlsRuntime::start(&Config { tls: TlsMode::Off, ..Default::default() });
    /// assert!(!off.status().enabled);
    /// ```
    pub fn start(config: &Config) -> Arc<Self> {
        Self::start_with(
            config,
            Arc::new(RcgenGenerator),
            SanSources::detect(config.tls_advertise_hostname),
            OffsetDateTime::now_utc(),
        )
    }

    /// The ladder with injected generator, host facts and clock. Rungs, in order: TLS off by
    /// configuration; the persisted self-signed certificate; an in-memory self-signed
    /// certificate; TLS disabled for this process. A configured operator PEM (and, when ACME
    /// issuance is unavailable, configured ACME domains) layers a primary certificate on top or
    /// records `degraded_from` and keeps serving self-signed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// use shadowcat::tls::selfsigned::{RcgenGenerator, SanSources};
    /// use shadowcat::tls::TlsRuntime;
    ///
    /// let rt = TlsRuntime::start_with(
    ///     &Config { db: "sqlite::memory:".into(), ..Default::default() },
    ///     Arc::new(RcgenGenerator),
    ///     SanSources::default(),
    ///     time::OffsetDateTime::now_utc(),
    /// );
    /// assert!(rt.status().enabled);
    /// ```
    pub fn start_with(
        config: &Config,
        generator: Arc<dyn CertGenerator>,
        sources: SanSources,
        now: OffsetDateTime,
    ) -> Arc<Self> {
        if config.tls == TlsMode::Off {
            return Arc::new(Self::off(config, None));
        }
        let settings = Settings::from_config(config, sources, generator);
        let mut state = State::default();

        let selfsigned = match selfsigned::load_or_generate(
            settings.tls_dir.as_deref(),
            &settings.tls_hostnames,
            &settings.sources,
            settings.generator.as_ref(),
            now,
            &settings.trash,
        ) {
            Ok(ss) => ss,
            Err(e) => {
                tracing::error!(error = %e, "self-signed certificate unavailable; TLS is disabled for this process, plain HTTP is unaffected");
                let mut off = Self::off(config, Some(e.to_string()));
                off.settings = settings;
                return Arc::new(off);
            }
        };
        match (&selfsigned.persist_error, settings.tls_dir.is_some()) {
            (Some(err), _) => {
                tracing::warn!(error = %err, "TLS directory not writable; serving an in-memory self-signed certificate that is regenerated on every start");
                state.last_error = Some(err.clone());
            }
            (None, false) => tracing::warn!(
                "no persistent location for the self-signed certificate (in-memory database and no tls_dir); it is regenerated on every start and browsers warn again each time"
            ),
            (None, true) => {}
        }
        if let Some(reason) = &selfsigned.regenerated {
            tracing::info!(?reason, fingerprint = %selfsigned.cert.fingerprint(), "self-signed certificate regenerated");
        }
        if !selfsigned.drift.is_empty() {
            tracing::info!(
                addresses = ?selfsigned.drift,
                "certificate does not list these local addresses; browsers show a name mismatch on top of the trust warning. A tls_hostnames entry puts that address on the certificate, and the certificate shown to internet players lists every tls_hostnames entry too, so add only an address you are willing to publish; or run --tls-regenerate"
            );
        }

        let public = match selfsigned::load_or_generate_public(
            settings.tls_dir.as_deref(),
            &settings.tls_hostnames,
            &settings.sources,
            None,
            settings.generator.as_ref(),
            now,
            &settings.trash,
        ) {
            Ok(public) => public,
            Err(e) => {
                tracing::error!(error = %e, "public-peer self-signed certificate unavailable; TLS is disabled for this process, plain HTTP is unaffected");
                let mut off = Self::off(config, Some(e.to_string()));
                off.settings = settings;
                return Arc::new(off);
            }
        };
        if let Some(err) = &public.persist_error {
            tracing::warn!(error = %err, "public-peer self-signed certificate could not be persisted; it is served from memory only");
            state.last_error = Some(err.clone());
        }

        let mut primary: Option<Arc<LoadedCert>> = None;
        if let Some(paths) = &settings.pem {
            state.pem_stamp = paths.stamp();
            match paths.load(now.unix_timestamp()) {
                Ok(cert) => {
                    warn_if_expiring(&cert, "operator certificate", now.unix_timestamp());
                    log_operator_coverage(&cert, "operator certificate loaded");
                    state.primary_source = Some(TlsSource::Operator);
                    primary = Some(Arc::new(cert));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "operator certificate unusable; serving the self-signed certificate");
                    state.degraded_from = Some(TlsSource::Operator);
                    state.last_error = Some(e.to_string());
                }
            }
        }
        if !config.acme_domains.is_empty() {
            let reason = acme_precheck(config)
                .err()
                .unwrap_or_else(|| ACME_UNAVAILABLE.to_string());
            tracing::warn!(error = %reason, "ACME disabled for this process");
            state.degraded_from = Some(TlsSource::Acme);
            state.last_error = Some(reason);
        }
        state.last_selfsigned_check = now.unix_timestamp();

        match Snapshot::new(primary, selfsigned.cert.clone(), Default::default())
            .and_then(|snapshot| snapshot.with_public_fallback(public.cert.clone()))
        {
            Ok(snapshot) => {
                let named = snapshot.primary().unwrap_or(snapshot.fallback());
                tracing::info!(
                    source = ?state.primary_source.unwrap_or(TlsSource::SelfSigned),
                    fingerprint = %named.fingerprint(),
                    fallback_fingerprint = %snapshot.fallback().fingerprint(),
                    public_fallback_fingerprint = %snapshot.public_fallback().fingerprint(),
                    "TLS certificates active"
                );
                Arc::new(Self {
                    resolver: Some(Arc::new(CertResolver::new(snapshot))),
                    state: Mutex::new(state),
                    settings,
                    external: Mutex::new(None),
                })
            }
            Err(e) => {
                tracing::error!(error = %e, "TLS configuration could not be built; TLS is disabled for this process, plain HTTP is unaffected");
                let mut off = Self::off(config, Some(e.to_string()));
                off.settings = settings;
                Arc::new(off)
            }
        }
    }

    /// The swap cell, `None` when TLS is off.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(shadowcat::tls::TlsRuntime::disabled().resolver().is_none());
    /// ```
    pub fn resolver(&self) -> Option<&Arc<CertResolver>> {
        self.resolver.as_ref()
    }

    /// The public status block.
    ///
    /// # Examples
    ///
    /// ```
    /// let s = shadowcat::tls::TlsRuntime::disabled().status();
    /// assert!(!s.enabled && s.source.is_none());
    /// ```
    pub fn status(&self) -> TlsStatus {
        self.status_at(OffsetDateTime::now_utc())
    }

    /// [`TlsRuntime::status`] evaluated at `now`: a primary certificate that has expired by then is
    /// reported as the self-signed fallback it is actually being replaced by
    /// ([`Snapshot::primary_at`]).
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// assert!(!rt.status_at(time::OffsetDateTime::now_utc()).enabled);
    /// ```
    pub fn status_at(&self, now: OffsetDateTime) -> TlsStatus {
        let alpn = ALPN_PROTOCOLS.iter().map(|s| s.to_string()).collect();
        let Some(resolver) = &self.resolver else {
            return TlsStatus {
                enabled: false,
                source: None,
                https_origins: self.settings.static_origins.clone(),
                alpn,
            };
        };
        let snapshot = resolver.snapshot();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let effective = snapshot.primary_at(now.unix_timestamp());
        let source = match (effective, state.primary_source) {
            (Some(_), Some(s)) => s,
            _ => TlsSource::SelfSigned,
        };
        let mut origins = self.settings.static_origins.clone();
        if let (Some(primary), TlsSource::Operator, false) =
            (effective, source, self.settings.has_public_url)
        {
            for name in primary.plain_dns_sans() {
                push_unique(&mut origins, origin_for(name, self.settings.bind_port));
            }
        }
        TlsStatus {
            enabled: true,
            source: Some(source),
            https_origins: origins,
            alpn,
        }
    }

    /// The admin-only status: [`TlsRuntime::status`] plus fingerprints, expiry and diagnostics.
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::start(&shadowcat::config::Config {
    ///     db: "sqlite::memory:".into(),
    ///     ..Default::default()
    /// });
    /// let admin = rt.admin_status();
    /// assert_eq!(admin.fingerprint, admin.fallback_fingerprint);
    /// ```
    pub fn admin_status(&self) -> TlsAdminStatus {
        self.admin_status_at(OffsetDateTime::now_utc())
    }

    /// [`TlsRuntime::admin_status`] evaluated at `now` (see [`TlsRuntime::status_at`]): the
    /// fingerprint and expiry are those of the certificate a named connection would actually
    /// receive at `now`.
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// assert!(rt.admin_status_at(time::OffsetDateTime::now_utc()).fingerprint.is_none());
    /// ```
    pub fn admin_status_at(&self, now: OffsetDateTime) -> TlsAdminStatus {
        let public = self.status_at(now);
        let Some(resolver) = &self.resolver else {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            return TlsAdminStatus {
                public,
                fingerprint: None,
                fallback_fingerprint: None,
                not_after: None,
                degraded_from: state.degraded_from,
                last_error: state.last_error.clone(),
            };
        };
        let snapshot = resolver.snapshot();
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let named = snapshot
            .primary_at(now.unix_timestamp())
            .unwrap_or(snapshot.fallback());
        TlsAdminStatus {
            public,
            fingerprint: Some(named.fingerprint().to_string()),
            fallback_fingerprint: Some(snapshot.fallback().fingerprint().to_string()),
            not_after: Some(named.not_after()),
            degraded_from: state.degraded_from,
            last_error: state.last_error.clone(),
        }
    }

    /// One maintenance pass at `now`: reload the operator PEM pair when its modification stamp
    /// changed (a failed reload keeps the previous good certificate and records the error), and
    /// once a day re-check the self-signed certificate's expiry (regenerating and hot-swapping it
    /// when [`selfsigned::regen_reason`] says so). Driven by [`TlsRuntime::spawn_maintenance`];
    /// public so a test can drive it with an injected clock.
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// rt.maintenance_tick(time::OffsetDateTime::now_utc()); // no-op when TLS is off
    /// ```
    pub fn maintenance_tick(&self, now: OffsetDateTime) {
        let Some(resolver) = &self.resolver else {
            return;
        };
        let now_unix = now.unix_timestamp();
        if let Some(paths) = &self.settings.pem {
            let stamp = paths.stamp();
            let changed = {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let changed = st.pem_stamp != stamp;
                st.pem_stamp = stamp;
                changed
            };
            if changed {
                match paths.load(now_unix) {
                    Ok(cert) => {
                        warn_if_expiring(&cert, "operator certificate", now_unix);
                        log_operator_coverage(&cert, "operator certificate reloaded");
                        let fingerprint = cert.fingerprint().to_string();
                        match resolver.update(|cur| cur.with_primary(Some(Arc::new(cert)))) {
                            Ok(()) => {
                                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                                st.primary_source = Some(TlsSource::Operator);
                                st.degraded_from = None;
                                st.last_error = None;
                                tracing::info!(%fingerprint, "operator certificate installed");
                            }
                            Err(e) => self.record_error(&e),
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "operator certificate reload failed; keeping the previous certificate");
                        self.record_error(&e);
                    }
                }
            }
        }
        let due = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            now_unix - st.last_selfsigned_check >= SELFSIGNED_CHECK_SECS
        };
        if due {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last_selfsigned_check = now_unix;
            self.refresh_selfsigned(resolver, now);
        }
        self.refresh_public(resolver, now, due);
    }

    /// Installs the reader of the router's public address. The maintenance tick compares it with
    /// the certificate shown to public peers on every pass.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// rt.set_external_source(Arc::new(|| Some("203.0.113.9".parse().unwrap())));
    /// ```
    pub fn set_external_source(&self, source: ExternalSource) {
        *self.external.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
    }

    /// The router's public address, when a source is installed and knows it.
    fn external_address(&self) -> Option<IpAddr> {
        let source = self
            .external
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        source()
    }

    /// Re-runs the policy of the certificate shown to public peers and swaps in a replacement
    /// when one was produced. A known external address the certificate lacks is checked on every
    /// pass (it is a comparison of two in-memory values); expiry and the hostname rules join the
    /// check only when `full` is set, the daily cadence of [`TlsRuntime::refresh_selfsigned`].
    fn refresh_public(&self, resolver: &CertResolver, now: OffsetDateTime, full: bool) {
        let external = self.external_address();
        let current = resolver.snapshot();
        let cert = current.public_fallback();
        let missing = external.is_some_and(|ip| !cert.san_entries().contains(&ip.to_string()));
        if !missing && !full {
            return;
        }
        let Some(reason) = selfsigned::regen_reason_public(
            cert,
            &self.settings.tls_hostnames,
            &self.settings.sources,
            external,
            now.unix_timestamp(),
        ) else {
            return;
        };
        match selfsigned::load_or_generate_public(
            self.settings.tls_dir.as_deref(),
            &self.settings.tls_hostnames,
            &self.settings.sources,
            external,
            self.settings.generator.as_ref(),
            now,
            &self.settings.trash,
        ) {
            Ok(replacement) => {
                if let Some(err) = &replacement.persist_error {
                    tracing::warn!(error = %err, "regenerated public-peer certificate could not be persisted; it is served from memory only");
                    self.record_error_text(err);
                }
                let fingerprint = replacement.cert.fingerprint().to_string();
                match resolver.update(|cur| cur.with_public_fallback(replacement.cert)) {
                    Ok(()) => {
                        tracing::info!(?reason, %fingerprint, "public-peer certificate regenerated");
                    }
                    Err(e) => self.record_error(&e),
                }
            }
            Err(e) => self.record_error(&e),
        }
    }

    /// Re-runs the self-signed policy and swaps in a replacement when one was produced.
    fn refresh_selfsigned(&self, resolver: &CertResolver, now: OffsetDateTime) {
        let current = resolver.snapshot();
        let Some(reason) = selfsigned::regen_reason(
            current.fallback(),
            &self.settings.tls_hostnames,
            &self.settings.sources,
            now.unix_timestamp(),
        ) else {
            return;
        };
        match selfsigned::load_or_generate(
            self.settings.tls_dir.as_deref(),
            &self.settings.tls_hostnames,
            &self.settings.sources,
            self.settings.generator.as_ref(),
            now,
            &self.settings.trash,
        ) {
            Ok(replacement) => {
                if let Some(err) = &replacement.persist_error {
                    tracing::warn!(error = %err, "regenerated self-signed certificate could not be persisted; it is served from memory only");
                    self.record_error_text(err);
                }
                let fingerprint = replacement.cert.fingerprint().to_string();
                match resolver.update(|cur| cur.with_fallback(replacement.cert)) {
                    Ok(()) => {
                        tracing::info!(?reason, %fingerprint, "self-signed certificate regenerated");
                    }
                    Err(e) => self.record_error(&e),
                }
            }
            Err(e) => self.record_error(&e),
        }
    }

    /// Records `e` as the latest TLS-side error.
    fn record_error(&self, e: &TlsError) {
        self.record_error_text(&e.to_string());
    }

    /// Records `text` as the latest TLS-side error.
    fn record_error_text(&self, text: &str) {
        self.state
            .lock()
            .unwrap_or_else(|s| s.into_inner())
            .last_error = Some(text.to_string());
    }

    /// Spawns the periodic maintenance loop (one pass per minute). The loop holds only a weak
    /// reference, so dropping the runtime ends it.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main] async fn main() {
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// shadowcat::tls::TlsRuntime::spawn_maintenance(&rt);
    /// # }
    /// ```
    pub fn spawn_maintenance(this: &Arc<Self>) {
        if this.resolver.is_none() {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(this);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(rt) = weak.upgrade() else { break };
                // The tick reads certificate files and may generate a key: blocking work that
                // must not run on a runtime worker.
                let _ = tokio::task::spawn_blocking(move || {
                    rt.maintenance_tick(OffsetDateTime::now_utc())
                })
                .await;
            }
        });
    }

    /// Human-readable startup lines for `bind`: trusted certificate source first with `https://`
    /// leading; self-signed lists both URLs with the trade-off labelled.
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = shadowcat::tls::TlsRuntime::disabled();
    /// let lines = rt.startup_lines("127.0.0.1:30000".parse().unwrap());
    /// assert!(lines.iter().any(|l| l.starts_with("http://127.0.0.1:30000")));
    /// ```
    pub fn startup_lines(&self, bind: SocketAddr) -> Vec<String> {
        let status = self.status();
        let mut hosts: Vec<String> = Vec::new();
        if bind.ip().is_unspecified() {
            hosts.push("localhost".into());
            for ip in &self.settings.sources.interface_ips {
                hosts.push(match ip {
                    std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                    v4 => v4.to_string(),
                });
            }
        } else {
            hosts.push(match bind.ip() {
                std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                v4 => v4.to_string(),
            });
        }
        let mut out = Vec::new();
        let trusted = matches!(status.source, Some(TlsSource::Operator | TlsSource::Acme));
        if status.enabled {
            for o in &status.https_origins {
                out.push(format!("{o}  (encrypted, trusted certificate)"));
            }
        }
        let port = bind.port();
        for h in &hosts {
            if status.enabled {
                let label = if trusted {
                    "encrypted"
                } else {
                    "encrypted, browser warns once"
                };
                out.push(format!("https://{h}:{port}  ({label})"));
            }
        }
        for h in &hosts {
            out.push(format!("http://{h}:{port}  (unencrypted, no warning)"));
        }
        out
    }
}

impl Settings {
    /// Derives the immutable inputs from `config`.
    fn from_config(
        config: &Config,
        sources: SanSources,
        generator: Arc<dyn CertGenerator>,
    ) -> Self {
        let bind_port = config
            .bind
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .unwrap_or(30000);
        let mut static_origins: Vec<String> = Vec::new();
        // The published origin is what `https_origin_of` accepts; a `public_url` it rejects
        // (which `Config::validate_tls` refuses at boot) publishes nothing and claims nothing.
        let public_origin = config
            .public_url
            .as_deref()
            .and_then(crate::config::https_origin_of);
        let has_public_url = public_origin.is_some();
        if let Some(origin) = public_origin {
            static_origins.push(origin);
        } else {
            for d in &config.acme_domains {
                let port = match config.acme_challenge {
                    AcmeChallenge::TlsAlpn01 => 443,
                    AcmeChallenge::Http01 => bind_port,
                };
                push_unique(
                    &mut static_origins,
                    origin_for(&d.to_ascii_lowercase(), port),
                );
            }
            for h in &config.tls_hostnames {
                if h.parse::<std::net::IpAddr>().is_err() && !h.starts_with("*.") {
                    push_unique(
                        &mut static_origins,
                        origin_for(&h.to_ascii_lowercase(), bind_port),
                    );
                }
            }
        }
        Self {
            tls_dir: config.tls_path(),
            tls_hostnames: config.tls_hostnames.clone(),
            pem: match (&config.tls_cert, &config.tls_key) {
                (Some(c), Some(k)) => Some(PemPaths {
                    cert: PathBuf::from(c),
                    key: PathBuf::from(k),
                }),
                _ => None,
            },
            static_origins,
            has_public_url,
            bind_port,
            sources,
            generator,
            trash: Trash::from_config(config),
        }
    }
}

/// Appends `item` unless already present.
fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// The `https://` origin for `host` on `port` (443 elided).
fn origin_for(host: &str, port: u16) -> String {
    if port == 443 {
        format!("https://{host}")
    } else {
        format!("https://{host}:{port}")
    }
}

/// Logs which names an operator certificate covers, so an operator sees at once that a name they
/// intend to use is missing from the certificate.
fn log_operator_coverage(cert: &LoadedCert, what: &str) {
    tracing::info!(names = ?cert.san_entries(), not_after = cert.not_after(), "{what}");
}

/// Warns when `cert` expires within the renewal window.
fn warn_if_expiring(cert: &LoadedCert, what: &str, now_unix: i64) {
    let left = cert.not_after() - now_unix;
    if left <= selfsigned::RENEW_WITHIN_DAYS * 86_400 {
        tracing::warn!(days_left = left / 86_400, "{what} expires soon");
    }
}

/// Reads and parses `acme_ca_root`, the only ACME precondition that needs file I/O. An
/// unreadable or not-a-certificate file disables ACME for the process (never a boot error).
fn acme_precheck(config: &Config) -> Result<(), String> {
    use rustls::pki_types::pem::PemObject;
    let Some(path) = &config.acme_ca_root else {
        return Ok(());
    };
    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("acme_ca_root {path}: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("acme_ca_root {path}: {e}"))?;
    if certs.is_empty() {
        return Err(format!("acme_ca_root {path}: no certificate found"));
    }
    Ok(())
}
