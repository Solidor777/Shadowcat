//! This module selects the certificate each TLS connection receives.
//!
//! It holds the immutable [`Snapshot`], its swap cell [`CertResolver`], and the two free
//! functions [`hello_phase`] / [`finish_phase`] that split one TLS accept into "read the
//! ClientHello and choose" and "complete the handshake with the chosen configuration".
//!
//! INVARIANT: the certificate presented, `ConnMeta.cert` and the HSTS decision all derive from ONE
//! [`Snapshot::select`] call on ONE `Arc<Snapshot>`. [`finish_phase`] receives that snapshot by
//! value and has no handle to the swap cell, so a second selection (or a re-read of the cell after
//! a hot swap) is impossible by construction.
//!
//! INVARIANT: the self-signed certificate a connection receives depends on [`PeerScope::of`] of the
//! accepted socket's peer address, decided in exactly one place ([`finish_phase`]). A snapshot
//! built without a public or host certificate serves that scope the shared fallback.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;

use super::{ring_provider, ConnMeta, Selected, TlsError, ALPN_PROTOCOLS};

/// ALPN protocol id of the TLS-ALPN-01 ACME validation handshake (RFC 8737).
pub const ACME_TLS_ALPN: &[u8] = b"acme-tls/1";

/// A certificate chain loaded and validated for serving, with the metadata selection needs cached
/// at load time so choosing a certificate is a comparison, not a parse.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{generate, SanSet};
///
/// let cert = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded;
/// assert!(cert.covers_name("localhost"));
/// assert_eq!(cert.fingerprint().len(), 64);
/// ```
#[derive(Debug)]
pub struct LoadedCert {
    /// The chain and signing key handed to rustls.
    key: Arc<CertifiedKey>,
    /// Lowercased DNS subject alternative names (may include a leading `*.` wildcard).
    dns_sans: Vec<String>,
    /// IP subject alternative names.
    ip_sans: Vec<std::net::IpAddr>,
    /// Start of the validity window, unix seconds.
    not_before: i64,
    /// End of the validity window, unix seconds.
    not_after: i64,
    /// Lowercase hex SHA-256 of the leaf DER.
    fingerprint: String,
}

impl LoadedCert {
    /// Builds a `LoadedCert` from a DER chain (leaf first) and its private key under the ring
    /// provider, verifying the key matches the leaf and parsing SANs and validity.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    /// use shadowcat::tls::resolver::LoadedCert;
    ///
    /// let g = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap();
    /// let again = LoadedCert::from_der(vec![g.cert_der.clone()], g.key_der.clone_key()).unwrap();
    /// assert_eq!(again.fingerprint(), g.loaded.fingerprint());
    /// ```
    pub fn from_der(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, TlsError> {
        let leaf = chain
            .first()
            .ok_or_else(|| TlsError::Cert("empty certificate chain".into()))?
            .clone();
        let certified = CertifiedKey::from_der(chain, key, &ring_provider()).map_err(|e| {
            TlsError::Cert(format!("private key does not match the certificate: {e}"))
        })?;
        let (_, parsed) = x509_parser::parse_x509_certificate(leaf.as_ref())
            .map_err(|e| TlsError::Cert(format!("unparsable certificate: {e}")))?;
        let mut dns_sans = Vec::new();
        let mut ip_sans = Vec::new();
        if let Ok(Some(ext)) = parsed.subject_alternative_name() {
            for name in &ext.value.general_names {
                match name {
                    x509_parser::extensions::GeneralName::DNSName(d) => {
                        dns_sans.push(d.to_ascii_lowercase())
                    }
                    x509_parser::extensions::GeneralName::IPAddress(bytes) => {
                        if let Some(ip) = ip_from_octets(bytes) {
                            ip_sans.push(ip);
                        }
                    }
                    _ => {}
                }
            }
        }
        let validity = parsed.validity();
        Ok(Self {
            key: Arc::new(certified),
            dns_sans,
            ip_sans,
            not_before: validity.not_before.timestamp(),
            not_after: validity.not_after.timestamp(),
            fingerprint: hex(ring::digest::digest(&ring::digest::SHA256, leaf.as_ref()).as_ref()),
        })
    }

    /// The rustls certificate chain + signing key.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let cert = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded;
    /// assert_eq!(cert.certified_key().cert.len(), 1);
    /// ```
    pub fn certified_key(&self) -> &Arc<CertifiedKey> {
        &self.key
    }

    /// Lowercase hex SHA-256 of the leaf certificate's DER encoding.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let cert = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded;
    /// assert!(cert.fingerprint().chars().all(|c| c.is_ascii_hexdigit()));
    /// ```
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// End of the validity window as unix seconds.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let cert = generate(&SanSet::default(), now).unwrap().loaded;
    /// assert!(cert.not_after() > now.unix_timestamp());
    /// ```
    pub fn not_after(&self) -> i64 {
        self.not_after
    }

    /// Non-wildcard DNS SANs, lowercase.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let sans = SanSet { dns: vec!["game.example".into()], ips: vec![] };
    /// let cert = generate(&sans, time::OffsetDateTime::now_utc()).unwrap().loaded;
    /// assert!(cert.plain_dns_sans().any(|d| d == "game.example"));
    /// ```
    pub fn plain_dns_sans(&self) -> impl Iterator<Item = &str> {
        self.dns_sans
            .iter()
            .map(String::as_str)
            .filter(|d| !d.starts_with("*."))
    }

    /// Every SAN as a comparable entry: DNS names lowercase, IP literals in canonical text.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let cert = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded;
    /// assert!(cert.san_entries().contains(&"localhost".to_string()));
    /// ```
    pub fn san_entries(&self) -> Vec<String> {
        self.dns_sans
            .iter()
            .cloned()
            .chain(self.ip_sans.iter().map(|ip| ip.to_string()))
            .collect()
    }

    /// Whether `now_unix` (seconds) lies inside the validity window.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let cert = generate(&SanSet::default(), now).unwrap().loaded;
    /// assert!(cert.is_valid_at(now.unix_timestamp()));
    /// assert!(!cert.is_valid_at(cert.not_after() + 1));
    /// ```
    pub fn is_valid_at(&self, now_unix: i64) -> bool {
        self.not_before <= now_unix && now_unix < self.not_after
    }

    /// Whether a DNS SNI value is covered: an exact SAN match, or a single-label wildcard
    /// (`*.example.com` covers `a.example.com`, not `a.b.example.com` and not `example.com`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let sans = SanSet { dns: vec!["*.game.example".into()], ips: vec![] };
    /// let cert = generate(&sans, time::OffsetDateTime::now_utc()).unwrap().loaded;
    /// assert!(cert.covers_name("a.game.example"));
    /// assert!(!cert.covers_name("a.b.game.example"));
    /// assert!(!cert.covers_name("game.example"));
    /// ```
    pub fn covers_name(&self, sni: &str) -> bool {
        let sni = sni.to_ascii_lowercase();
        self.dns_sans.iter().any(|san| {
            if let Some(suffix) = san.strip_prefix("*.") {
                sni.split_once('.')
                    .is_some_and(|(label, rest)| !label.is_empty() && rest == suffix)
            } else {
                *san == sni
            }
        })
    }
}

/// Parses raw SAN IP octets (4 or 16 bytes) into an address.
fn ip_from_octets(bytes: &[u8]) -> Option<std::net::IpAddr> {
    match bytes.len() {
        4 => <[u8; 4]>::try_from(bytes).ok().map(std::net::IpAddr::from),
        16 => <[u8; 16]>::try_from(bytes).ok().map(std::net::IpAddr::from),
        _ => None,
    }
}

/// Lowercase hex encoding.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A resolver that always presents one fixed certificate. Never consulted to CHOOSE (selection
/// happens once in [`Snapshot::select`]); rustls needs a resolver object to build a config.
#[derive(Debug)]
struct FixedCert(Arc<CertifiedKey>);

impl ResolvesServerCert for FixedCert {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// Presents the TLS-ALPN-01 validation certificate registered for the connection's SNI.
#[derive(Debug)]
struct ChallengeCerts(BTreeMap<String, Arc<CertifiedKey>>);

impl ResolvesServerCert for ChallengeCerts {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let sni = client_hello.server_name()?.to_ascii_lowercase();
        self.0.get(&sni).cloned()
    }
}

/// Builds a server configuration under the explicit ring provider (never rustls' feature-based
/// auto-selection) with the given resolver and ALPN list.
fn build_config(
    resolver: Arc<dyn ResolvesServerCert>,
    alpn: Vec<Vec<u8>>,
) -> Result<Arc<ServerConfig>, TlsError> {
    let mut cfg = ServerConfig::builder_with_provider(ring_provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Cert(format!("protocol versions: {e}")))?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    cfg.alpn_protocols = alpn;
    Ok(Arc::new(cfg))
}

/// The pre-built rustls configurations of one snapshot.
#[derive(Debug)]
struct Configs {
    /// Serves the CA-trusted certificate; `None` when the snapshot has no primary.
    primary: Option<Arc<ServerConfig>>,
    /// Serves the self-signed certificate to a [`PeerScope::Lan`] peer.
    fallback: Arc<ServerConfig>,
    /// Serves the self-signed certificate to a [`PeerScope::Public`] peer; `None` when the
    /// snapshot has one self-signed certificate for every peer.
    public_fallback: Option<Arc<ServerConfig>>,
    /// Serves the self-signed certificate to a [`PeerScope::Host`] peer; `None` when the
    /// snapshot has no host-machine certificate.
    host_fallback: Option<Arc<ServerConfig>>,
    /// Serves TLS-ALPN-01 validation certificates; `None` when no challenge is published.
    challenge: Option<Arc<ServerConfig>>,
}

/// An immutable view of everything certificate selection reads. A hot swap installs a NEW
/// snapshot in the [`CertResolver`]; a snapshot is never mutated.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use shadowcat::tls::resolver::Snapshot;
/// use shadowcat::tls::selfsigned::{generate, SanSet};
/// use shadowcat::tls::Selected;
///
/// let fallback = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
/// let snap = Snapshot::new(None, fallback, Default::default()).unwrap();
/// assert_eq!(snap.select(Some("localhost"), &[]), Selected::Fallback);
/// ```
#[derive(Debug)]
pub struct Snapshot {
    /// The CA-trusted certificate (operator PEM or ACME-issued), when one is loaded.
    primary: Option<Arc<LoadedCert>>,
    /// The self-signed certificate a [`PeerScope::Lan`] peer receives; always present.
    fallback: Arc<LoadedCert>,
    /// The self-signed certificate a [`PeerScope::Public`] peer receives; `None` means every peer
    /// receives `fallback`.
    public_fallback: Option<Arc<LoadedCert>>,
    /// The self-signed certificate a [`PeerScope::Host`] peer receives: it names only localhost,
    /// 127.0.0.1 and ::1. `None` means a host peer receives the fallback every peer shares.
    host_fallback: Option<Arc<LoadedCert>>,
    /// TLS-ALPN-01 validation certificates keyed by lowercase SNI.
    challenges: BTreeMap<String, Arc<CertifiedKey>>,
    /// Pre-built configurations, one per [`Selected`] outcome.
    configs: Configs,
}

impl Snapshot {
    /// Builds a snapshot and its per-outcome rustls configurations.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// assert!(Snapshot::new(None, fb, Default::default()).is_ok());
    /// ```
    pub fn new(
        primary: Option<Arc<LoadedCert>>,
        fallback: Arc<LoadedCert>,
        challenges: BTreeMap<String, Arc<CertifiedKey>>,
    ) -> Result<Self, TlsError> {
        Self::build(primary, fallback, None, None, challenges)
    }

    /// Builds a snapshot from every certificate slot and its rustls configurations. Every
    /// constructor and every `with_*` derivation goes through here, so a derived snapshot cannot
    /// drop a slot.
    fn build(
        primary: Option<Arc<LoadedCert>>,
        fallback: Arc<LoadedCert>,
        public_fallback: Option<Arc<LoadedCert>>,
        host_fallback: Option<Arc<LoadedCert>>,
        challenges: BTreeMap<String, Arc<CertifiedKey>>,
    ) -> Result<Self, TlsError> {
        let serving_alpn = || {
            ALPN_PROTOCOLS
                .iter()
                .map(|p| p.as_bytes().to_vec())
                .collect::<Vec<_>>()
        };
        let primary_cfg = match &primary {
            Some(p) => Some(build_config(
                Arc::new(FixedCert(p.key.clone())),
                serving_alpn(),
            )?),
            None => None,
        };
        let fallback_cfg = build_config(Arc::new(FixedCert(fallback.key.clone())), serving_alpn())?;
        let public_cfg = match &public_fallback {
            Some(p) => Some(build_config(
                Arc::new(FixedCert(p.key.clone())),
                serving_alpn(),
            )?),
            None => None,
        };
        let host_cfg = match &host_fallback {
            Some(h) => Some(build_config(
                Arc::new(FixedCert(h.key.clone())),
                serving_alpn(),
            )?),
            None => None,
        };
        let challenge_cfg = if challenges.is_empty() {
            None
        } else {
            Some(build_config(
                Arc::new(ChallengeCerts(challenges.clone())),
                vec![ACME_TLS_ALPN.to_vec()],
            )?)
        };
        Ok(Self {
            primary,
            fallback,
            public_fallback,
            host_fallback,
            challenges,
            configs: Configs {
                primary: primary_cfg,
                fallback: fallback_cfg,
                public_fallback: public_cfg,
                host_fallback: host_cfg,
                challenge: challenge_cfg,
            },
        })
    }

    /// A NEW snapshot with `primary` replaced, sharing this snapshot's fallback and challenges.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let fb = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb, Default::default()).unwrap();
    /// let p = Arc::new(generate(&SanSet { dns: vec!["a.example".into()], ips: vec![] }, now).unwrap().loaded);
    /// let next = snap.with_primary(Some(p)).unwrap();
    /// assert!(next.primary().is_some() && snap.primary().is_none());
    /// ```
    pub fn with_primary(&self, primary: Option<Arc<LoadedCert>>) -> Result<Self, TlsError> {
        Self::build(
            primary,
            self.fallback.clone(),
            self.public_fallback.clone(),
            self.host_fallback.clone(),
            self.challenges.clone(),
        )
    }

    /// A NEW snapshot with the self-signed fallback replaced.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let a = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let b = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let snap = Snapshot::new(None, a.clone(), Default::default()).unwrap();
    /// let next = snap.with_fallback(b.clone()).unwrap();
    /// assert_eq!(next.fallback().fingerprint(), b.fingerprint());
    /// ```
    pub fn with_fallback(&self, fallback: Arc<LoadedCert>) -> Result<Self, TlsError> {
        Self::build(
            self.primary.clone(),
            fallback,
            self.public_fallback.clone(),
            self.host_fallback.clone(),
            self.challenges.clone(),
        )
    }

    /// A NEW snapshot whose [`PeerScope::Public`] peers receive `public_fallback` instead of the
    /// self-signed fallback.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let lan = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let public = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let snap = Snapshot::new(None, lan, Default::default()).unwrap();
    /// let split = snap.with_public_fallback(public.clone()).unwrap();
    /// assert_eq!(split.public_fallback().fingerprint(), public.fingerprint());
    /// assert_eq!(snap.public_fallback().fingerprint(), snap.fallback().fingerprint());
    /// ```
    pub fn with_public_fallback(&self, public_fallback: Arc<LoadedCert>) -> Result<Self, TlsError> {
        Self::build(
            self.primary.clone(),
            self.fallback.clone(),
            Some(public_fallback),
            self.host_fallback.clone(),
            self.challenges.clone(),
        )
    }

    /// A NEW snapshot whose [`PeerScope::Host`] peers receive `host_fallback` instead of the
    /// self-signed fallback.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let lan = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let host = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let snap = Snapshot::new(None, lan, Default::default()).unwrap();
    /// let split = snap.with_host_fallback(host.clone()).unwrap();
    /// assert_eq!(split.host_fallback().fingerprint(), host.fingerprint());
    /// assert_eq!(snap.host_fallback().fingerprint(), snap.fallback().fingerprint());
    /// ```
    pub fn with_host_fallback(&self, host_fallback: Arc<LoadedCert>) -> Result<Self, TlsError> {
        Self::build(
            self.primary.clone(),
            self.fallback.clone(),
            self.public_fallback.clone(),
            Some(host_fallback),
            self.challenges.clone(),
        )
    }

    /// A NEW snapshot with the TLS-ALPN-01 validation certificates replaced.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb, Default::default()).unwrap();
    /// assert!(snap.with_challenges(Default::default()).is_ok());
    /// ```
    pub fn with_challenges(
        &self,
        challenges: BTreeMap<String, Arc<CertifiedKey>>,
    ) -> Result<Self, TlsError> {
        Self::build(
            self.primary.clone(),
            self.fallback.clone(),
            self.public_fallback.clone(),
            self.host_fallback.clone(),
            challenges,
        )
    }

    /// The CA-trusted certificate, when loaded.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// assert!(Snapshot::new(None, fb, Default::default()).unwrap().primary().is_none());
    /// ```
    pub fn primary(&self) -> Option<&Arc<LoadedCert>> {
        self.primary.as_ref()
    }

    /// The self-signed fallback certificate a [`PeerScope::Lan`] peer receives.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb.clone(), Default::default()).unwrap();
    /// assert_eq!(snap.fallback().fingerprint(), fb.fingerprint());
    /// ```
    pub fn fallback(&self) -> &Arc<LoadedCert> {
        &self.fallback
    }

    /// The self-signed certificate a [`PeerScope::Public`] peer receives: the one installed by
    /// [`Snapshot::with_public_fallback`], else the fallback every peer shares.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb.clone(), Default::default()).unwrap();
    /// assert_eq!(snap.public_fallback().fingerprint(), fb.fingerprint());
    /// ```
    pub fn public_fallback(&self) -> &Arc<LoadedCert> {
        self.public_fallback.as_ref().unwrap_or(&self.fallback)
    }

    /// The self-signed certificate a [`PeerScope::Host`] peer receives: the one installed by
    /// [`Snapshot::with_host_fallback`], else the fallback every peer shares.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb.clone(), Default::default()).unwrap();
    /// assert_eq!(snap.host_fallback().fingerprint(), fb.fingerprint());
    /// ```
    pub fn host_fallback(&self) -> &Arc<LoadedCert> {
        self.host_fallback.as_ref().unwrap_or(&self.fallback)
    }

    /// The self-signed certificate a peer of `scope` receives. [`Snapshot::config_for`] and the
    /// admin status both read the certificate through this one function, so what an admin is
    /// told is the certificate that admin's browser gets.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::{PeerScope, Snapshot};
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb.clone(), Default::default()).unwrap();
    /// assert_eq!(snap.self_signed_for(PeerScope::Host).fingerprint(), fb.fingerprint());
    /// ```
    pub fn self_signed_for(&self, scope: PeerScope) -> &Arc<LoadedCert> {
        match scope {
            PeerScope::Lan => &self.fallback,
            PeerScope::Public => self.public_fallback(),
            PeerScope::Host => self.host_fallback(),
        }
    }

    /// THE certificate decision, pure over `(snapshot, sni, alpn, now)`. ALPN `acme-tls/1` with a
    /// published challenge for the SNI selects the challenge certificate; else a loaded, currently
    /// valid primary whose DNS SANs cover the SNI is chosen; everything else (IP-literal access,
    /// no SNI, an uncovered name, an expired or absent primary) gets the self-signed fallback,
    /// which carries the IP SANs.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    /// use shadowcat::tls::Selected;
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let fb = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let p = Arc::new(generate(&SanSet { dns: vec!["game.example".into()], ips: vec![] }, now).unwrap().loaded);
    /// let snap = Snapshot::new(Some(p), fb, Default::default()).unwrap();
    /// assert_eq!(snap.select(Some("game.example"), &[]), Selected::Primary);
    /// assert_eq!(snap.select(None, &[]), Selected::Fallback);
    /// ```
    pub fn select(&self, sni: Option<&str>, alpn: &[&[u8]]) -> Selected {
        self.select_at(sni, alpn, unix_now())
    }

    /// [`Snapshot::select`] with an explicit clock reading (unix seconds).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    /// use shadowcat::tls::Selected;
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let fb = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let p = Arc::new(generate(&SanSet { dns: vec!["game.example".into()], ips: vec![] }, now).unwrap().loaded);
    /// let far = p.not_after() + 10;
    /// let snap = Snapshot::new(Some(p), fb, Default::default()).unwrap();
    /// assert_eq!(snap.select_at(Some("game.example"), &[], far), Selected::Fallback);
    /// ```
    pub fn select_at(&self, sni: Option<&str>, alpn: &[&[u8]], now_unix: i64) -> Selected {
        let sni = sni.filter(|s| s.parse::<std::net::IpAddr>().is_err());
        if alpn.contains(&ACME_TLS_ALPN) {
            if let Some(name) = sni {
                if self.challenges.contains_key(&name.to_ascii_lowercase()) {
                    return Selected::Challenge;
                }
            }
        }
        if let (Some(primary), Some(name)) = (self.primary_at(now_unix), sni) {
            if primary.covers_name(name) {
                return Selected::Primary;
            }
        }
        Selected::Fallback
    }

    /// The primary certificate when it is loaded AND valid at `now_unix`; an expired primary is
    /// treated as absent. [`Snapshot::select_at`] and the status views both read the primary
    /// through this one function, so what is reported is what is served.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::Snapshot;
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let fb = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let p = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let far = p.not_after() + 10;
    /// let snap = Snapshot::new(Some(p), fb, Default::default()).unwrap();
    /// assert!(snap.primary_at(now.unix_timestamp()).is_some());
    /// assert!(snap.primary_at(far).is_none());
    /// ```
    pub fn primary_at(&self, now_unix: i64) -> Option<&Arc<LoadedCert>> {
        self.primary.as_ref().filter(|p| p.is_valid_at(now_unix))
    }

    /// The rustls configuration serving `selected` to a peer of `scope`. A `Primary`/`Challenge`
    /// request against a snapshot without one falls back to the self-signed configuration for
    /// `scope`; [`Snapshot::select`] never returns such an outcome for the snapshot that produced
    /// it. The self-signed outcome is the only one `scope` changes: the operator and challenge
    /// certificates are CA-facing and carry no interface address.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::{PeerScope, Snapshot};
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    /// use shadowcat::tls::Selected;
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let snap = Snapshot::new(None, fb, Default::default()).unwrap();
    /// let config = snap.config_for(Selected::Fallback, PeerScope::Lan);
    /// assert_eq!(config.alpn_protocols, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
    /// ```
    pub fn config_for(&self, selected: Selected, scope: PeerScope) -> Arc<ServerConfig> {
        let self_signed = || {
            let scoped = match scope {
                PeerScope::Lan => None,
                PeerScope::Public => self.configs.public_fallback.as_ref(),
                PeerScope::Host => self.configs.host_fallback.as_ref(),
            };
            scoped.unwrap_or(&self.configs.fallback).clone()
        };
        match selected {
            Selected::Primary => self.configs.primary.clone().unwrap_or_else(self_signed),
            Selected::Challenge => self.configs.challenge.clone().unwrap_or_else(self_signed),
            Selected::Fallback => self_signed(),
        }
    }
}

/// Which self-signed certificate a peer is shown, decided once from its socket address and from
/// whether that address is a listed trusted proxy.
///
/// The self-signed certificate is presented before any authentication, so only a
/// [`PeerScope::Lan`] peer may read a local interface address from it. A listed trusted proxy
/// fronts players whose addresses are unknown, so it is `Public`. A loopback peer is the host
/// machine itself (the GM's own browser) or a tunnel agent running on it; it is `Host` and
/// receives a certificate that names only localhost, 127.0.0.1 and ::1, which serves the host's
/// browser and shows a same-machine tunnel's players nothing of the host's network. Remaining
/// limit: a router that rewrites the source address of forwarded connections to a private LAN
/// address, or a passthrough proxy on another LAN machine that is not a listed trusted proxy, is
/// indistinguishable from a LAN player and is shown the LAN certificate.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::resolver::PeerScope;
///
/// assert_eq!(PeerScope::of("192.168.1.20".parse().unwrap(), false), PeerScope::Lan);
/// assert_eq!(PeerScope::of("203.0.113.9".parse().unwrap(), false), PeerScope::Public);
/// assert_eq!(PeerScope::of("127.0.0.1".parse().unwrap(), false), PeerScope::Host);
/// assert_eq!(PeerScope::of("127.0.0.1".parse().unwrap(), true), PeerScope::Public);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerScope {
    /// Private, link-local or unique-local: a peer on the host's own network.
    Lan,
    /// Any other address, including carrier-grade NAT (`100.64.0.0/10`), which no local network
    /// owns, and every listed trusted proxy.
    Public,
    /// Loopback: the host machine itself.
    Host,
}

impl PeerScope {
    /// The scope of a peer at `ip` (an IPv4-mapped IPv6 address counts as its IPv4 address).
    /// `trusted_proxy` is `Config::is_trusted_proxy` for the peer: every player behind such an
    /// address shares it, so the peer is `Public` even on loopback.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::resolver::PeerScope;
    ///
    /// assert_eq!(PeerScope::of("::ffff:10.0.0.4".parse().unwrap(), false), PeerScope::Lan);
    /// assert_eq!(PeerScope::of("fd00::1".parse().unwrap(), false), PeerScope::Lan);
    /// assert_eq!(PeerScope::of("::1".parse().unwrap(), false), PeerScope::Host);
    /// assert_eq!(PeerScope::of("fd00::1".parse().unwrap(), true), PeerScope::Public);
    /// assert_eq!(PeerScope::of("2001:db8::1".parse().unwrap(), false), PeerScope::Public);
    /// ```
    pub fn of(ip: std::net::IpAddr, trusted_proxy: bool) -> Self {
        if trusted_proxy {
            return PeerScope::Public;
        }
        let ip = ip.to_canonical();
        if ip.is_loopback() {
            return PeerScope::Host;
        }
        let lan = match ip {
            std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                let first = v6.segments()[0];
                first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
            }
        };
        if lan {
            PeerScope::Lan
        } else {
            PeerScope::Public
        }
    }
}

/// Current wall-clock time as unix seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The swap cell holding the current [`Snapshot`]. A hot swap ([`CertResolver::update`]) replaces the whole `Arc`; a
/// connection takes the `Arc` once and uses it for both handshake phases.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use shadowcat::tls::resolver::{CertResolver, Snapshot};
/// use shadowcat::tls::selfsigned::{generate, SanSet};
///
/// let now = time::OffsetDateTime::now_utc();
/// let a = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
/// let b = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
/// let cell = CertResolver::new(Snapshot::new(None, a.clone(), Default::default()).unwrap());
/// let before = cell.snapshot();
/// let b2 = b.clone();
/// cell.update(|_| Snapshot::new(None, b2, Default::default())).unwrap();
/// assert_eq!(before.fallback().fingerprint(), a.fingerprint());
/// assert_eq!(cell.snapshot().fallback().fingerprint(), b.fingerprint());
/// ```
#[derive(Debug)]
pub struct CertResolver {
    /// The current snapshot.
    cell: RwLock<Arc<Snapshot>>,
}

impl CertResolver {
    /// A cell holding `snapshot`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::{CertResolver, Snapshot};
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let cell = CertResolver::new(Snapshot::new(None, fb, Default::default()).unwrap());
    /// assert!(cell.snapshot().primary().is_none());
    /// ```
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            cell: RwLock::new(Arc::new(snapshot)),
        }
    }

    /// The current snapshot.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::{CertResolver, Snapshot};
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let fb = Arc::new(generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded);
    /// let cell = CertResolver::new(Snapshot::new(None, fb, Default::default()).unwrap());
    /// let a = cell.snapshot();
    /// let b = cell.snapshot();
    /// assert!(Arc::ptr_eq(&a, &b));
    /// ```
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.cell.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Derives the next snapshot from the CURRENT one and installs it, both under the cell's
    /// write lock, so concurrent writers each build on the other's result (a read-then-install
    /// pair would let the later install discard the earlier one's change). `derive` must be cheap
    /// and non-blocking: it runs while every reader waits. On `Err` the cell is unchanged.
    /// Connections already past their hello keep the snapshot they took.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::tls::resolver::{CertResolver, Snapshot};
    /// use shadowcat::tls::selfsigned::{generate, SanSet};
    ///
    /// let now = time::OffsetDateTime::now_utc();
    /// let fb = Arc::new(generate(&SanSet::default(), now).unwrap().loaded);
    /// let cell = CertResolver::new(Snapshot::new(None, fb.clone(), Default::default()).unwrap());
    /// let p = Arc::new(generate(&SanSet { dns: vec!["x.example".into()], ips: vec![] }, now).unwrap().loaded);
    /// cell.update(|cur| cur.with_primary(Some(p))).unwrap();
    /// assert!(cell.snapshot().primary().is_some());
    /// ```
    pub fn update<E>(
        &self,
        derive: impl FnOnce(&Snapshot) -> Result<Snapshot, E>,
    ) -> Result<(), E> {
        let mut guard = self.cell.write().unwrap_or_else(|e| e.into_inner());
        let next = derive(&guard)?;
        *guard = Arc::new(next);
        Ok(())
    }
}

/// The hello phase of a TLS accept: reads the ClientHello from `start` and calls
/// [`Snapshot::select`] exactly once. Touches no shared mutable state.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use shadowcat::tls::resolver::{hello_phase, Snapshot};
/// use tokio_rustls::LazyConfigAcceptor;
///
/// # async fn demo(snapshot: Arc<Snapshot>, io: tokio::net::TcpStream) -> std::io::Result<()> {
/// let start = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), io).await?;
/// let selected = hello_phase(&start, &snapshot);
/// # let _ = selected;
/// # Ok(()) }
/// ```
pub fn hello_phase<IO>(start: &StartHandshake<IO>, snapshot: &Snapshot) -> Selected
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let hello = start.client_hello();
    let offered: Vec<&[u8]> = hello.alpn().map(|it| it.collect()).unwrap_or_default();
    snapshot.select(hello.server_name(), &offered)
}

/// The finish phase of a TLS accept: completes the handshake with the configuration `selected`
/// maps to in `snapshot` for the scope of `peer` ([`PeerScope::of`], the one place a peer address
/// picks a certificate; `trusted_proxy` is `Config::is_trusted_proxy` for `peer`) and records the single selection outcome in the returned [`ConnMeta`].
/// Takes the snapshot by value and has no access to the swap cell, so it cannot select a second
/// time.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use shadowcat::tls::resolver::{finish_phase, hello_phase, Snapshot};
/// use tokio_rustls::LazyConfigAcceptor;
///
/// # async fn demo(snapshot: Arc<Snapshot>, io: tokio::net::TcpStream, peer: std::net::SocketAddr) -> std::io::Result<()> {
/// let start = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), io).await?;
/// let selected = hello_phase(&start, &snapshot);
/// let (_stream, meta) = finish_phase(start, snapshot, selected, peer, false).await?;
/// assert!(meta.tls);
/// # Ok(()) }
/// ```
pub async fn finish_phase<IO>(
    start: StartHandshake<IO>,
    snapshot: Arc<Snapshot>,
    selected: Selected,
    peer: SocketAddr,
    trusted_proxy: bool,
) -> std::io::Result<(TlsStream<IO>, ConnMeta)>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let config = snapshot.config_for(selected, PeerScope::of(peer.ip(), trusted_proxy));
    let stream = start.into_stream(config).await?;
    Ok((stream, ConnMeta::tls(peer, selected)))
}
