//! This module manages the persisted self-signed leaf certificate.
//!
//! It holds the leaf certificate's attributes, SAN policy, reuse/regeneration rules, and
//! the interface classifier that decides which local addresses it advertises.
//!
//! Algorithm: ECDSA P-256 (`rcgen`'s default key algorithm under the ring backend). Validity
//! window: `not_before = now - 1 day` (clock-skew tolerance), `not_after = now + 397 days` (the
//! longest lifetime mainstream browsers accept for a leaf). The attributes are set explicitly
//! because `rcgen::CertificateParams::default()` yields a 1975-4096 validity, no key usages and no
//! basicConstraints, none of which is acceptable for a served certificate.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};

use super::keyfile;
use super::resolver::LoadedCert;
use super::TlsError;
use crate::trash::Trash;

/// File name of the persisted certificate under the TLS directory.
pub const CERT_FILE: &str = "selfsigned-cert.pem";
/// File name of the persisted private key under the TLS directory.
pub const KEY_FILE: &str = "selfsigned-key.pem";
/// File name of the persisted certificate shown to public peers (no interface addresses).
pub const PUBLIC_CERT_FILE: &str = "selfsigned-public-cert.pem";
/// File name of the private key of the certificate shown to public peers.
pub const PUBLIC_KEY_FILE: &str = "selfsigned-public-key.pem";
/// Days of remaining validity at or below which the certificate is regenerated.
pub const RENEW_WITHIN_DAYS: i64 = 30;
/// Lifetime of a freshly generated certificate, in days.
pub const VALIDITY_DAYS: i64 = 397;
/// Common name of the generated certificate.
pub const COMMON_NAME: &str = "Shadowcat server";

/// The DNS names and IP literals a generated certificate advertises.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::SanSet;
///
/// let sans = SanSet { dns: vec!["game.example".into()], ips: vec!["192.0.2.7".parse().unwrap()] };
/// assert_eq!(sans.dns.len(), 1);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SanSet {
    /// DNS names, including an optional leading `*.` wildcard.
    pub dns: Vec<String>,
    /// IP literals.
    pub ips: Vec<IpAddr>,
}

/// Local facts the SAN list is derived from. Injected so the policy is testable without a real
/// host.
///
/// PRIVACY: a self-signed certificate is presented to every peer that completes a TLS handshake,
/// before any authentication, and machine hostnames commonly embed a person's name. The hostname
/// (and its `.local` form) therefore enters a certificate only when
/// [`SanSources::advertise_hostname`] is set (`Config::tls_advertise_hostname`); it is detected
/// regardless so a certificate persisted by an earlier run that DID list it is recognised and
/// replaced ([`regen_reason`]). Interface addresses describe the host's network layout, so they
/// enter only the certificate shown to a LAN peer ([`SanSet::assemble`]); the certificate shown to
/// a public peer lists none ([`SanSet::assemble_public`]).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::SanSources;
///
/// let s = SanSources { machine_hostname: Some("table".into()), ..Default::default() };
/// assert_eq!(s.machine_hostname.as_deref(), Some("table"));
/// assert!(!s.advertise_hostname);
/// ```
#[derive(Debug, Default, Clone)]
pub struct SanSources {
    /// The machine's hostname.
    pub machine_hostname: Option<String>,
    /// Whether the machine hostname and its `<hostname>.local` form are listed in the certificate.
    pub advertise_hostname: bool,
    /// Eligible interface addresses (see [`eligible_interface_ips`]).
    pub interface_ips: Vec<IpAddr>,
}

impl SanSources {
    /// Reads the machine hostname and enumerates eligible interface addresses from the OS. A
    /// failure or an empty enumeration is logged and leaves that source empty: enumeration
    /// failure is never fatal for TLS.
    ///
    /// # Examples
    ///
    /// ```
    /// let s = shadowcat::tls::selfsigned::SanSources::detect(false);
    /// assert!(s.interface_ips.iter().all(|ip| !ip.is_loopback()));
    /// assert!(!s.advertise_hostname);
    /// ```
    pub fn detect(advertise_hostname: bool) -> Self {
        let machine_hostname = gethostname::gethostname().into_string().ok();
        let infos = enumerate_interfaces();
        if infos.is_empty() {
            tracing::warn!(
                "network interface enumeration returned nothing; the self-signed certificate lists only localhost, loopback and tls_hostnames"
            );
        }
        Self {
            machine_hostname,
            advertise_hostname,
            interface_ips: eligible_interface_ips(&infos),
        }
    }

    /// The lowercase DNS names the machine hostname contributes when advertised: the hostname and,
    /// for a single-label hostname, its `<hostname>.local` multicast-DNS form. Empty when there is
    /// no usable hostname (missing, an IP literal, not a valid DNS name, or `localhost`, which is always listed anyway).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::SanSources;
    ///
    /// let s = SanSources { machine_hostname: Some("Table-PC".into()), ..Default::default() };
    /// assert_eq!(s.machine_names(), vec!["table-pc".to_string(), "table-pc.local".to_string()]);
    /// let fqdn = SanSources { machine_hostname: Some("host.lan.example".into()), ..Default::default() };
    /// assert_eq!(fqdn.machine_names(), vec!["host.lan.example".to_string()]);
    /// ```
    pub fn machine_names(&self) -> Vec<String> {
        let Some(NormalisedEntry::Dns(host)) =
            self.machine_hostname.as_deref().and_then(normalise_entry)
        else {
            return Vec::new();
        };
        // A name failing host validation (non-ASCII, bad label) can never be placed in the
        // certificate, so it is neither listed nor required: requiring it would regenerate the
        // certificate at every check.
        if host == "localhost" || !crate::config::is_valid_host_entry(&host, false) {
            return Vec::new();
        }
        if host.contains('.') {
            vec![host]
        } else {
            let mdns = format!("{host}.local");
            vec![host, mdns]
        }
    }
}

/// One certificate host entry after normalisation: the single definition of "the same name" for
/// SAN assembly and for the regeneration check, so the two cannot disagree on case, whitespace or
/// IP spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NormalisedEntry {
    /// An IP literal in canonical form.
    Ip(IpAddr),
    /// A lowercase DNS name (possibly a leading `*.` wildcard).
    Dns(String),
}

impl NormalisedEntry {
    /// The string form `LoadedCert::san_entries` reports for this entry.
    fn key(&self) -> String {
        match self {
            NormalisedEntry::Ip(ip) => ip.to_string(),
            NormalisedEntry::Dns(name) => name.clone(),
        }
    }
}

/// Trims `entry` and classifies it as an IP literal or a lowercase DNS name; `None` for blank
/// input.
fn normalise_entry(entry: &str) -> Option<NormalisedEntry> {
    let e = entry.trim();
    if e.is_empty() {
        return None;
    }
    Some(match e.parse::<IpAddr>() {
        Ok(ip) => NormalisedEntry::Ip(ip),
        Err(_) => NormalisedEntry::Dns(e.to_ascii_lowercase()),
    })
}

/// One network interface as the classifier sees it.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::InterfaceInfo;
///
/// let i = InterfaceInfo { names: vec!["eth0".into()], addrs: vec!["192.0.2.7".parse().unwrap()] };
/// assert_eq!(i.addrs.len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct InterfaceInfo {
    /// Every name the OS gives the interface (system name, friendly name, description).
    pub names: Vec<String>,
    /// Every address assigned to it.
    pub addrs: Vec<IpAddr>,
}

/// Enumerates the host's interfaces via `netdev`. Empty on failure.
///
/// # Examples
///
/// ```
/// let _ = shadowcat::tls::selfsigned::enumerate_interfaces();
/// ```
pub fn enumerate_interfaces() -> Vec<InterfaceInfo> {
    netdev::get_interfaces()
        .into_iter()
        .map(|i| {
            let mut names = vec![i.name.clone()];
            names.extend(i.friendly_name.clone());
            names.extend(i.description.clone());
            let addrs = i
                .ipv4
                .iter()
                .map(|n| IpAddr::V4(n.addr()))
                .chain(i.ipv6.iter().map(|n| IpAddr::V6(n.addr())))
                .collect();
            InterfaceInfo { names, addrs }
        })
        .collect()
}

/// Whether an interface name matches the virtual-adapter list (`docker*`, `br-*`, `veth*`,
/// `virbr*`, `vmnet*`, `vboxnet*`, Windows `vEthernet*`), case-insensitively. A heuristic:
/// addresses on such adapters are unreachable from other machines, so advertising them only
/// bloats the certificate.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::is_virtual_adapter_name;
///
/// assert!(is_virtual_adapter_name("docker0"));
/// assert!(is_virtual_adapter_name("vEthernet (WSL)"));
/// assert!(!is_virtual_adapter_name("eth0"));
/// ```
pub fn is_virtual_adapter_name(name: &str) -> bool {
    const PREFIXES: [&str; 7] = [
        "docker",
        "br-",
        "veth",
        "virbr",
        "vmnet",
        "vboxnet",
        "vethernet",
    ];
    let lower = name.to_ascii_lowercase();
    PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// Whether `ip` may be advertised: IPv4 private or global (loopback, link-local, unspecified,
/// multicast and broadcast are excluded); IPv6 global unicast or unique-local (loopback,
/// link-local, unspecified and multicast are excluded).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::is_eligible_ip;
///
/// assert!(is_eligible_ip("192.168.1.20".parse().unwrap()));
/// assert!(!is_eligible_ip("169.254.10.1".parse().unwrap()));
/// assert!(is_eligible_ip("fd00::1".parse().unwrap()));
/// assert!(!is_eligible_ip("fe80::1".parse().unwrap()));
/// ```
pub fn is_eligible_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => eligible_v4(v4),
        IpAddr::V6(v6) => eligible_v6(v6),
    }
}

/// IPv4 half of [`is_eligible_ip`].
fn eligible_v4(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast())
}

/// IPv6 half of [`is_eligible_ip`]: global unicast (`2000::/3`) or unique-local (`fc00::/7`).
fn eligible_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let global = first & 0xe000 == 0x2000;
    let unique_local = first & 0xfe00 == 0xfc00;
    global || unique_local
}

/// The addresses of `infos` eligible for the certificate: not on a virtual adapter and passing
/// [`is_eligible_ip`], deduplicated in first-seen order.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{eligible_interface_ips, InterfaceInfo};
///
/// let infos = vec![
///     InterfaceInfo { names: vec!["eth0".into()], addrs: vec!["192.168.1.20".parse().unwrap(), "fe80::1".parse().unwrap()] },
///     InterfaceInfo { names: vec!["docker0".into()], addrs: vec!["172.17.0.1".parse().unwrap()] },
/// ];
/// assert_eq!(eligible_interface_ips(&infos), vec!["192.168.1.20".parse::<std::net::IpAddr>().unwrap()]);
/// ```
pub fn eligible_interface_ips(infos: &[InterfaceInfo]) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::new();
    for info in infos {
        if info.names.iter().any(|n| is_virtual_adapter_name(n)) {
            continue;
        }
        for ip in &info.addrs {
            if is_eligible_ip(*ip) && !out.contains(ip) {
                out.push(*ip);
            }
        }
    }
    out
}

impl SanSet {
    /// Builds the SAN list: `localhost`, loopback addresses, the machine hostname and its `.local`
    /// form ONLY when `sources.advertise_hostname` is set (see [`SanSources`]), every configured
    /// `tls_hostnames` entry (IP literals routed to `ips`) and the eligible interface addresses,
    /// deduplicated. Pure over its inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{SanSet, SanSources};
    ///
    /// let mut sources = SanSources {
    ///     machine_hostname: Some("table".into()),
    ///     interface_ips: vec!["192.168.1.20".parse().unwrap()],
    ///     ..Default::default()
    /// };
    /// let sans = SanSet::assemble(&["game.example".to_string()], &sources);
    /// assert!(sans.dns.contains(&"localhost".to_string()));
    /// assert!(!sans.dns.contains(&"table".to_string()), "the hostname is opt-in");
    /// assert!(sans.ips.contains(&"192.168.1.20".parse().unwrap()));
    /// sources.advertise_hostname = true;
    /// let opted_in = SanSet::assemble(&[], &sources);
    /// assert!(opted_in.dns.contains(&"table".to_string()));
    /// assert!(opted_in.dns.contains(&"table.local".to_string()));
    /// ```
    pub fn assemble(tls_hostnames: &[String], sources: &SanSources) -> Self {
        let mut set = SanSet {
            dns: vec!["localhost".into()],
            ips: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        };
        if sources.advertise_hostname {
            for name in sources.machine_names() {
                set.add_entry(Some(&name));
            }
        }
        for h in tls_hostnames {
            set.add_entry(Some(h));
        }
        for ip in &sources.interface_ips {
            if !set.ips.contains(ip) {
                set.ips.push(*ip);
            }
        }
        set
    }

    /// Builds the SAN list of the certificate shown to a public peer: the same as
    /// [`SanSet::assemble`] with NO interface address, plus `external` (the router's public
    /// address, the one the host hands out) when it is known.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{SanSet, SanSources};
    ///
    /// let sources = SanSources {
    ///     interface_ips: vec!["192.168.1.20".parse().unwrap()],
    ///     ..Default::default()
    /// };
    /// let external = "203.0.113.9".parse().unwrap();
    /// let sans = SanSet::assemble_public(&[], &sources, Some(external));
    /// assert!(sans.ips.contains(&external));
    /// assert!(!sans.ips.contains(&"192.168.1.20".parse().unwrap()));
    /// ```
    pub fn assemble_public(
        tls_hostnames: &[String],
        sources: &SanSources,
        external: Option<IpAddr>,
    ) -> Self {
        let mut set = Self::assemble(
            tls_hostnames,
            &SanSources {
                interface_ips: Vec::new(),
                ..sources.clone()
            },
        );
        if let Some(ip) = external {
            if !set.ips.contains(&ip) {
                set.ips.push(ip);
            }
        }
        set
    }

    /// Adds one text entry as an IP literal or a lowercase DNS name, skipping duplicates and
    /// empty input.
    fn add_entry(&mut self, entry: Option<&str>) {
        match entry.and_then(normalise_entry) {
            Some(NormalisedEntry::Ip(ip)) if !self.ips.contains(&ip) => self.ips.push(ip),
            Some(NormalisedEntry::Dns(name)) if !self.dns.contains(&name) => self.dns.push(name),
            Some(_) | None => {}
        }
    }
}

/// A freshly generated certificate in every representation the callers need.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{generate, SanSet};
///
/// let g = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap();
/// assert!(g.cert_pem.starts_with("-----BEGIN CERTIFICATE-----"));
/// ```
#[derive(Debug)]
pub struct Generated {
    /// The certificate loaded for serving.
    pub loaded: LoadedCert,
    /// Leaf certificate DER.
    pub cert_der: CertificateDer<'static>,
    /// PKCS#8 private key DER.
    pub key_der: PrivateKeyDer<'static>,
    /// Leaf certificate PEM.
    pub cert_pem: String,
    /// PKCS#8 private key PEM.
    pub key_pem: String,
}

/// Generates a self-signed leaf for `sans` at time `now`. `localhost` and the loopback
/// addresses are always added. A DNS entry rcgen rejects is skipped with a warning, so a bad
/// entry can never fail generation; only a genuine key-generation or signing fault does.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{generate, SanSet};
///
/// let sans = SanSet { dns: vec!["game.example".into()], ips: vec!["192.0.2.7".parse().unwrap()] };
/// let g = generate(&sans, time::OffsetDateTime::now_utc()).unwrap();
/// let entries = g.loaded.san_entries();
/// assert!(entries.contains(&"game.example".to_string()));
/// assert!(entries.contains(&"192.0.2.7".to_string()));
/// assert!(entries.contains(&"127.0.0.1".to_string()));
/// ```
pub fn generate(sans: &SanSet, now: OffsetDateTime) -> Result<Generated, TlsError> {
    let base = SanSet::assemble(&[], &SanSources::default());
    let mut all = base;
    for d in &sans.dns {
        all.add_entry(Some(d));
    }
    for ip in &sans.ips {
        if !all.ips.contains(ip) {
            all.ips.push(*ip);
        }
    }

    let mut params = CertificateParams::default();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(VALIDITY_DAYS);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.is_ca = IsCa::ExplicitNoCa;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, COMMON_NAME);
    params.distinguished_name = dn;
    params.subject_alt_names = Vec::new();
    for d in &all.dns {
        match d.clone().try_into() {
            Ok(name) => params.subject_alt_names.push(SanType::DnsName(name)),
            Err(e) => tracing::warn!(entry = %d, error = %e, "certificate name rejected; skipped"),
        }
    }
    for ip in &all.ips {
        params.subject_alt_names.push(SanType::IpAddress(*ip));
    }

    let key =
        KeyPair::generate().map_err(|e| TlsError::Generate(format!("key generation: {e}")))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| TlsError::Generate(format!("signing: {e}")))?;
    let cert_der = cert.der().clone();
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let loaded = LoadedCert::from_der(vec![cert_der.clone()], key_der.clone_key())?;
    Ok(Generated {
        loaded,
        cert_der,
        key_der,
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Produces certificates. The default is [`RcgenGenerator`]; the seam exists so the boot ladder's
/// "generation failed" rung can be exercised with a generator that fails.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{CertGenerator, RcgenGenerator, SanSet};
///
/// let g = RcgenGenerator.generate(&SanSet::default(), time::OffsetDateTime::now_utc());
/// assert!(g.is_ok());
/// ```
pub trait CertGenerator: Send + Sync {
    /// Generates a certificate advertising `sans`, valid from about `now`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::tls::selfsigned::{CertGenerator, RcgenGenerator, SanSet};
    ///
    /// let generated = RcgenGenerator
    ///     .generate(&SanSet::default(), time::OffsetDateTime::now_utc())
    ///     .unwrap();
    /// assert!(generated.cert_pem.contains("BEGIN CERTIFICATE"));
    /// ```
    fn generate(&self, sans: &SanSet, now: OffsetDateTime) -> Result<Generated, TlsError>;
}

/// The production generator: [`generate`].
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{CertGenerator, RcgenGenerator, SanSet};
///
/// assert!(RcgenGenerator.generate(&SanSet::default(), time::OffsetDateTime::now_utc()).is_ok());
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct RcgenGenerator;

impl CertGenerator for RcgenGenerator {
    fn generate(&self, sans: &SanSet, now: OffsetDateTime) -> Result<Generated, TlsError> {
        generate(sans, now)
    }
}

/// Why a stored certificate is replaced.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::RegenReason;
///
/// assert_eq!(RegenReason::NearExpiry, RegenReason::NearExpiry);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegenReason {
    /// `not_after` is within [`RENEW_WITHIN_DAYS`] days (or already past).
    NearExpiry,
    /// The stored files are missing halves, unreadable or do not parse.
    Unusable,
    /// A configured `tls_hostnames` entry is absent from the certificate's SANs.
    MissingHostname(String),
    /// The certificate lists the machine hostname (or its `.local` form) although
    /// `tls_advertise_hostname` is off and `tls_hostnames` does not name it: a certificate
    /// persisted before the opt-in existed, or under the opt-in since switched off.
    MachineHostnameListed,
    /// `tls_advertise_hostname` is on and the certificate lacks the machine hostname.
    MachineHostnameMissing,
    /// The certificate shown to public peers lacks the router's current external address. An
    /// address that left the router's reach is never a reason: only the one players must use now.
    MissingExternalAddress(IpAddr),
}

/// Which of the two persisted self-signed certificates an operation handles.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// The certificate shown to a LAN peer: lists the eligible interface addresses.
    Lan,
    /// The certificate shown to a public peer: lists no interface address, and lists `external`
    /// (the router's public address) when it is known.
    Public {
        /// The router's public address, when the port-mapping runtime knows it.
        external: Option<IpAddr>,
    },
}

impl Kind {
    /// The persisted certificate file name.
    fn cert_file(self) -> &'static str {
        match self {
            Kind::Lan => CERT_FILE,
            Kind::Public { .. } => PUBLIC_CERT_FILE,
        }
    }

    /// The persisted key file name.
    fn key_file(self) -> &'static str {
        match self {
            Kind::Lan => KEY_FILE,
            Kind::Public { .. } => PUBLIC_KEY_FILE,
        }
    }

    /// The SAN list this kind of certificate carries.
    fn assemble(self, tls_hostnames: &[String], sources: &SanSources) -> SanSet {
        match self {
            Kind::Lan => SanSet::assemble(tls_hostnames, sources),
            Kind::Public { external } => SanSet::assemble_public(tls_hostnames, sources, external),
        }
    }

    /// Eligible interface addresses `cert` lacks (always empty for a public certificate, which
    /// lists none by design).
    fn drift(self, cert: &LoadedCert, sources: &SanSources) -> Vec<IpAddr> {
        match self {
            Kind::Lan => drifted_addresses(cert, &sources.interface_ips),
            Kind::Public { .. } => Vec::new(),
        }
    }
}

/// Decides whether a stored certificate must be replaced: near expiry, a configured
/// `tls_hostnames` entry missing from its SANs, or disagreement with the machine-hostname opt-in
/// (`sources.advertise_hostname`): a certificate that lists the machine hostname while the opt-in
/// is off is replaced so the hostname stops being shown to peers, and one lacking it while the
/// opt-in is on is replaced so the operator's choice takes effect. A hostname the operator also
/// named in `tls_hostnames` is theirs and never counts as "listed". A NEW interface address is
/// deliberately NOT a reason (VPN/WSL/docker/DHCP churn would re-warn every player).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{generate, regen_reason, RegenReason, SanSet, SanSources};
///
/// let now = time::OffsetDateTime::now_utc();
/// let cert = generate(&SanSet::default(), now).unwrap().loaded;
/// let sources = SanSources::default();
/// assert_eq!(regen_reason(&cert, &[], &sources, now.unix_timestamp()), None);
/// assert_eq!(
///     regen_reason(&cert, &["game.example".to_string()], &sources, now.unix_timestamp()),
///     Some(RegenReason::MissingHostname("game.example".into()))
/// );
/// ```
pub fn regen_reason(
    cert: &LoadedCert,
    tls_hostnames: &[String],
    sources: &SanSources,
    now_unix: i64,
) -> Option<RegenReason> {
    regen_reason_for(Kind::Lan, cert, tls_hostnames, sources, now_unix)
}

/// [`regen_reason`] for the certificate shown to public peers: the same expiry, `tls_hostnames`
/// and machine-hostname rules, plus [`RegenReason::MissingExternalAddress`] when `external` is
/// known and the certificate does not list it. A changed external address replaces the
/// certificate although a changed interface address never does, because the players' origin
/// changes with the external address: a browser asks again for a new address regardless, so the
/// replacement re-warns nobody who was not already warned.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{generate, regen_reason_public, RegenReason, SanSet, SanSources};
///
/// let now = time::OffsetDateTime::now_utc();
/// let cert = generate(&SanSet::default(), now).unwrap().loaded;
/// let external: std::net::IpAddr = "203.0.113.9".parse().unwrap();
/// let sources = SanSources::default();
/// assert_eq!(regen_reason_public(&cert, &[], &sources, None, now.unix_timestamp()), None);
/// assert_eq!(
///     regen_reason_public(&cert, &[], &sources, Some(external), now.unix_timestamp()),
///     Some(RegenReason::MissingExternalAddress(external))
/// );
/// ```
pub fn regen_reason_public(
    cert: &LoadedCert,
    tls_hostnames: &[String],
    sources: &SanSources,
    external: Option<IpAddr>,
    now_unix: i64,
) -> Option<RegenReason> {
    regen_reason_for(
        Kind::Public { external },
        cert,
        tls_hostnames,
        sources,
        now_unix,
    )
}

/// The single definition of "this stored certificate must be replaced" for both kinds.
fn regen_reason_for(
    kind: Kind,
    cert: &LoadedCert,
    tls_hostnames: &[String],
    sources: &SanSources,
    now_unix: i64,
) -> Option<RegenReason> {
    if cert.not_after() - now_unix <= RENEW_WITHIN_DAYS * 86_400 {
        return Some(RegenReason::NearExpiry);
    }
    let entries = cert.san_entries();
    let listed = |key: &str| entries.iter().any(|e| e == key);
    for h in tls_hostnames {
        if let Some(entry) = normalise_entry(h) {
            if !listed(&entry.key()) {
                return Some(RegenReason::MissingHostname(h.clone()));
            }
        }
    }
    let configured: Vec<String> = tls_hostnames
        .iter()
        .filter_map(|h| normalise_entry(h))
        .map(|e| e.key())
        .collect();
    let machine: Vec<String> = sources
        .machine_names()
        .into_iter()
        .filter(|n| !configured.contains(n))
        .collect();
    if sources.advertise_hostname {
        if machine.iter().any(|n| !listed(n)) {
            return Some(RegenReason::MachineHostnameMissing);
        }
    } else if machine.iter().any(|n| listed(n)) {
        return Some(RegenReason::MachineHostnameListed);
    }
    if let Kind::Public {
        external: Some(ip), ..
    } = kind
    {
        if !listed(&ip.to_string()) {
            return Some(RegenReason::MissingExternalAddress(ip));
        }
    }
    None
}

/// Eligible addresses missing from `cert`'s SANs (logged once as drift, never a regeneration
/// trigger).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{drifted_addresses, generate, SanSet};
///
/// let cert = generate(&SanSet::default(), time::OffsetDateTime::now_utc()).unwrap().loaded;
/// let lan: std::net::IpAddr = "192.168.1.20".parse().unwrap();
/// assert_eq!(drifted_addresses(&cert, &[lan]), vec![lan]);
/// ```
pub fn drifted_addresses(cert: &LoadedCert, eligible: &[IpAddr]) -> Vec<IpAddr> {
    let entries = cert.san_entries();
    eligible
        .iter()
        .copied()
        .filter(|ip| !entries.contains(&ip.to_string()))
        .collect()
}

/// The outcome of [`load_or_generate`].
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{load_or_generate, RcgenGenerator, SanSources};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let trash = Trash::new(std::path::PathBuf::from("trash"), TrashRetention::Disabled);
/// let out = load_or_generate(None, &[], &SanSources::default(), &RcgenGenerator, time::OffsetDateTime::now_utc(), &trash).unwrap();
/// assert!(!out.persisted);
/// ```
#[derive(Debug)]
pub struct SelfSigned {
    /// The certificate to serve.
    pub cert: Arc<LoadedCert>,
    /// Whether the certificate is backed by files under the TLS directory.
    pub persisted: bool,
    /// Why persistence was skipped or failed (`None` when persisted, or when no directory exists
    /// at all by configuration).
    pub persist_error: Option<String>,
    /// Set when a stored certificate was replaced during this call.
    pub regenerated: Option<RegenReason>,
    /// Eligible interface addresses the certificate does not list.
    pub drift: Vec<IpAddr>,
}

/// Reads the persisted certificate pair from `dir`.
fn read_persisted(dir: &Path, kind: Kind) -> Result<LoadedCert, TlsError> {
    let cert_path = dir.join(kind.cert_file());
    let key_path = dir.join(kind.key_file());
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_path)
        .map_err(|e| TlsError::Pem(format!("{}: {e}", cert_path.display())))?
        .collect::<Result<_, _>>()
        .map_err(|e| TlsError::Pem(format!("{}: {e}", cert_path.display())))?;
    let key = PrivateKeyDer::from_pem_file(&key_path)
        .map_err(|e| TlsError::Pem(format!("{}: {e}", key_path.display())))?;
    LoadedCert::from_der(chain, key)
}

/// Writes a generated pair under `dir` (key protected owner-only).
fn persist(dir: &Path, kind: Kind, g: &Generated, trash: &Trash) -> Result<(), TlsError> {
    let key_path = dir.join(kind.key_file());
    let cert_path = dir.join(kind.cert_file());
    keyfile::write_private(&key_path, g.key_pem.as_bytes(), trash)
        .map_err(|e| TlsError::Io(format!("{}: {e}", key_path.display())))?;
    keyfile::write_private(&cert_path, g.cert_pem.as_bytes(), trash)
        .map_err(|e| TlsError::Io(format!("{}: {e}", cert_path.display())))?;
    Ok(())
}

/// The ladder's self-signed rungs: reuse the certificate persisted under `dir` unless
/// [`regen_reason`] or an unusable file says otherwise; generate (and persist) a replacement;
/// when `dir` is `None` or unwritable serve an in-memory certificate for this process. Only a
/// genuine generation fault is an error.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{load_or_generate, RcgenGenerator, SanSources};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let now = time::OffsetDateTime::now_utc();
/// let first = load_or_generate(Some(dir.path()), &[], &SanSources::default(), &RcgenGenerator, now, &trash).unwrap();
/// let second = load_or_generate(Some(dir.path()), &[], &SanSources::default(), &RcgenGenerator, now, &trash).unwrap();
/// assert_eq!(first.cert.fingerprint(), second.cert.fingerprint());
/// ```
pub fn load_or_generate(
    dir: Option<&Path>,
    tls_hostnames: &[String],
    sources: &SanSources,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<SelfSigned, TlsError> {
    load_or_generate_kind(
        Kind::Lan,
        dir,
        tls_hostnames,
        sources,
        generator,
        now,
        trash,
    )
}

/// [`load_or_generate`] for the certificate shown to public peers: persisted beside the LAN one,
/// listing no interface address, and listing `external` when it is known. It follows the same
/// reuse and regeneration rules ([`regen_reason_public`]).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{load_or_generate_public, RcgenGenerator, SanSources};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let now = time::OffsetDateTime::now_utc();
/// let external = "203.0.113.9".parse().unwrap();
/// let out = load_or_generate_public(Some(dir.path()), &[], &SanSources::default(), Some(external), &RcgenGenerator, now, &trash).unwrap();
/// assert!(out.cert.san_entries().contains(&"203.0.113.9".to_string()));
/// ```
pub fn load_or_generate_public(
    dir: Option<&Path>,
    tls_hostnames: &[String],
    sources: &SanSources,
    external: Option<IpAddr>,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<SelfSigned, TlsError> {
    load_or_generate_kind(
        Kind::Public { external },
        dir,
        tls_hostnames,
        sources,
        generator,
        now,
        trash,
    )
}

/// The one reuse-or-replace path both certificates share.
fn load_or_generate_kind(
    kind: Kind,
    dir: Option<&Path>,
    tls_hostnames: &[String],
    sources: &SanSources,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<SelfSigned, TlsError> {
    let mut regenerated = None;
    if let Some(dir) = dir {
        if dir.join(kind.cert_file()).exists() || dir.join(kind.key_file()).exists() {
            match read_persisted(dir, kind) {
                Ok(existing) => {
                    match regen_reason_for(
                        kind,
                        &existing,
                        tls_hostnames,
                        sources,
                        now.unix_timestamp(),
                    ) {
                        None => {
                            let drift = kind.drift(&existing, sources);
                            return Ok(SelfSigned {
                                cert: Arc::new(existing),
                                persisted: true,
                                persist_error: None,
                                regenerated: None,
                                drift,
                            });
                        }
                        Some(reason) => regenerated = Some(reason),
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "stored self-signed certificate is unusable; regenerating");
                    regenerated = Some(RegenReason::Unusable);
                }
            }
        }
    }
    let sans = kind.assemble(tls_hostnames, sources);
    let g = generator.generate(&sans, now)?;
    let (persisted, persist_error) = match dir {
        Some(dir) => match persist(dir, kind, &g, trash) {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        },
        None => (false, None),
    };
    Ok(SelfSigned {
        cert: Arc::new(g.loaded),
        persisted,
        persist_error,
        regenerated,
        drift: Vec::new(),
    })
}

/// The directory a persisted certificate lives in, for diagnostics.
///
/// # Examples
///
/// ```
/// let p = shadowcat::tls::selfsigned::cert_path(std::path::Path::new("tls"));
/// assert!(p.ends_with("selfsigned-cert.pem"));
/// ```
pub fn cert_path(dir: &Path) -> PathBuf {
    dir.join(CERT_FILE)
}

/// One-shot `--tls-regenerate`: unconditionally generates a new certificate advertising the
/// current SAN set and persists it under `dir`.
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{force_regenerate, load_or_generate, RcgenGenerator, SanSources};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let now = time::OffsetDateTime::now_utc();
/// let a = load_or_generate(Some(dir.path()), &[], &SanSources::default(), &RcgenGenerator, now, &trash).unwrap();
/// let b = force_regenerate(dir.path(), &[], &SanSources::default(), &RcgenGenerator, now, &trash).unwrap();
/// assert_ne!(a.cert.fingerprint(), b.fingerprint());
/// ```
pub fn force_regenerate(
    dir: &Path,
    tls_hostnames: &[String],
    sources: &SanSources,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<Arc<LoadedCert>, TlsError> {
    force_regenerate_kind(
        Kind::Lan,
        dir,
        tls_hostnames,
        sources,
        generator,
        now,
        trash,
    )
}

/// [`force_regenerate`] for the certificate shown to public peers; `external` is the router's
/// public address when known (the one-shot command runs without a port-mapping runtime, so it
/// passes `None` and the running server adds the address on its next check).
///
/// # Examples
///
/// ```
/// use shadowcat::tls::selfsigned::{force_regenerate_public, RcgenGenerator, SanSources};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let now = time::OffsetDateTime::now_utc();
/// let cert = force_regenerate_public(dir.path(), &[], &SanSources::default(), None, &RcgenGenerator, now, &trash).unwrap();
/// assert!(cert.san_entries().contains(&"localhost".to_string()));
/// ```
pub fn force_regenerate_public(
    dir: &Path,
    tls_hostnames: &[String],
    sources: &SanSources,
    external: Option<IpAddr>,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<Arc<LoadedCert>, TlsError> {
    force_regenerate_kind(
        Kind::Public { external },
        dir,
        tls_hostnames,
        sources,
        generator,
        now,
        trash,
    )
}

/// The one unconditional-replacement path both certificates share.
fn force_regenerate_kind(
    kind: Kind,
    dir: &Path,
    tls_hostnames: &[String],
    sources: &SanSources,
    generator: &dyn CertGenerator,
    now: OffsetDateTime,
    trash: &Trash,
) -> Result<Arc<LoadedCert>, TlsError> {
    let sans = kind.assemble(tls_hostnames, sources);
    let g = generator.generate(&sans, now)?;
    persist(dir, kind, &g, trash)?;
    Ok(Arc::new(g.loaded))
}
