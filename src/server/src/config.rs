//! This module layers the server configuration: `Cli` flags override `SHADOWCAT_*` environment
//! variables, which override the TOML file, which overrides the defaults.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::net::{SocketAddr, ToSocketAddrs};

use clap::Parser;
use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

/// Built-in HTTPS mode (`Config.tls`).
///
/// # Examples
///
/// ```
/// use shadowcat::config::TlsMode;
///
/// assert_eq!(TlsMode::default(), TlsMode::Auto);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lower")]
pub enum TlsMode {
    /// The listener answers TLS on the same port as plain HTTP.
    #[default]
    Auto,
    /// No TLS: a connection whose first byte is a TLS record is dropped.
    Off,
}

/// ACME challenge type (`Config.acme_challenge`).
///
/// # Examples
///
/// ```
/// use shadowcat::config::AcmeChallenge;
///
/// assert_eq!(AcmeChallenge::default(), AcmeChallenge::TlsAlpn01);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
pub enum AcmeChallenge {
    /// Validation certificate answered on the TLS port (`acme-tls/1` ALPN); CA validates on 443.
    #[default]
    #[serde(rename = "tls-alpn-01")]
    #[value(name = "tls-alpn-01")]
    TlsAlpn01,
    /// Token served over plain HTTP at `/.well-known/acme-challenge/`; CA validates on 80.
    #[serde(rename = "http-01")]
    #[value(name = "http-01")]
    Http01,
}

/// Default ACME directory (Let's Encrypt production).
pub const DEFAULT_ACME_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// CLI flags. Every field is optional so it only overrides lower layers when
/// explicitly provided.
///
/// # Examples
///
/// ```
/// use shadowcat::config::{Cli, Config};
///
/// let cli = Cli { bind: Some("0.0.0.0:9000".into()), ..Default::default() };
/// let cfg = Config::load(cli).expect("config layering");
/// assert_eq!(cfg.bind, "0.0.0.0:9000");
/// ```
#[derive(Parser, Debug, Default)]
#[command(name = "shadowcat")]
pub struct Cli {
    /// Listen address override (`host:port`); wins over every other layer.
    #[arg(long)]
    pub bind: Option<String>,
    /// SQLite database file path override.
    #[arg(long)]
    pub db: Option<String>,
    /// TOML config file path (default `shadowcat.toml`; a missing file is ignored).
    #[arg(long)]
    pub config: Option<String>,
    /// Headless first-admin bootstrap: username override (pairs with `--admin-password`).
    #[arg(long)]
    pub admin_user: Option<String>,
    /// Headless first-admin bootstrap: password override (pairs with `--admin-user`).
    #[arg(long)]
    pub admin_password: Option<String>,
    /// Setup-window policy override: `auto` | `off` | `required` | an explicit token.
    #[arg(long)]
    pub setup_token: Option<String>,
    /// Session-cookie signing-key override (base64, >= 64 bytes decoded).
    #[arg(long)]
    pub session_key: Option<String>,
    /// Asset storage root override (default: sibling `assets/` beside the db file).
    #[arg(long)]
    pub assets_dir: Option<String>,
    /// Installed-module discovery root override (default: sibling `modules/`).
    #[arg(long)]
    pub modules_dir: Option<String>,
    /// Rolling log-file output root override (default: sibling `logs/`).
    #[arg(long)]
    pub log_dir: Option<String>,
    /// In-server backup output root override (default: sibling `backups/`).
    #[arg(long)]
    pub backups_dir: Option<String>,
    /// Keep at least this many of the newest in-app backups (`Config.backup_keep_last`).
    #[arg(long)]
    pub backup_keep_last: Option<u32>,
    /// Keep in-app backups newer than this many days (`Config.backup_keep_days`).
    #[arg(long)]
    pub backup_keep_days: Option<u32>,
    /// Trash retention override: a positive number of days, `never` or `disabled`.
    #[arg(long)]
    pub trash_retention: Option<String>,
    /// Trash storage root override (default: sibling `trash/` beside the db file).
    #[arg(long)]
    pub trash_dir: Option<String>,
    /// One-shot: snapshot the resolved db + assets into this directory, print
    /// a summary, and exit before the server would otherwise start.
    #[arg(long)]
    pub backup_to: Option<String>,
    /// One-shot: restore a prior `--backup-to` directory over the resolved db
    /// + assets, print a summary, and exit. Never starts the server.
    #[arg(long)]
    pub restore_from: Option<String>,
    /// Required to let `--backup-to` overwrite a non-empty output directory,
    /// or `--restore-from` overwrite an existing destination db/assets dir.
    #[arg(long)]
    pub force: bool,
    /// Keep the uploaded original beside the converted canonical file
    /// (`Config.retain_originals`); `--retain-originals false` discards it.
    #[arg(long)]
    pub retain_originals: Option<bool>,
    /// Declares that a reverse proxy fronts this bind on the same host (`Config.behind_proxy`).
    /// A loopback bind fronted this way is internet-reachable even though the bind address
    /// itself is not — set this so the setup-token policy treats the deployment as exposed.
    /// Never inferred automatically. The per-request scheme (cookie flavour, HSTS) is learned
    /// from `X-Forwarded-Proto` sent by a peer listed in `trusted_proxies`.
    #[arg(long)]
    pub behind_proxy: Option<bool>,
    /// Built-in HTTPS mode override (`auto` | `off`).
    #[arg(long, value_enum)]
    pub tls: Option<TlsMode>,
    /// Operator TLS certificate chain (PEM). Pairs with `--tls-key`.
    #[arg(long)]
    pub tls_cert: Option<String>,
    /// Operator TLS private key (PEM). Pairs with `--tls-cert`.
    #[arg(long)]
    pub tls_key: Option<String>,
    /// Extra DNS names / IP literals the self-signed certificate must cover (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub tls_hostnames: Option<Vec<String>>,
    /// List this machine's hostname (and `<hostname>.local`) in the self-signed certificate
    /// (`Config.tls_advertise_hostname`). Off by default: the certificate is shown to every peer
    /// before authentication and hostnames often carry a person's name.
    #[arg(long)]
    pub tls_advertise_hostname: Option<bool>,
    /// TLS state directory override (default: sibling `tls/` beside the db file).
    #[arg(long)]
    pub tls_dir: Option<String>,
    /// Public `https://` origin players reach this server at (published in `/api/config`).
    #[arg(long)]
    pub public_url: Option<String>,
    /// DNS names to obtain a certificate for through ACME (comma separated).
    #[arg(long, value_delimiter = ',')]
    pub acme_domains: Option<Vec<String>>,
    /// PEM file with the private ACME CA's root (or intermediate + root) to trust.
    #[arg(long)]
    pub acme_ca_root: Option<String>,
    /// ACME account contact e-mail.
    #[arg(long)]
    pub acme_email: Option<String>,
    /// Confirms the ACME CA's terms of service are accepted.
    #[arg(long)]
    pub acme_terms_agreed: Option<bool>,
    /// ACME challenge type (`tls-alpn-01` | `http-01`).
    #[arg(long, value_enum)]
    pub acme_challenge: Option<AcmeChallenge>,
    /// ACME directory URL (must be `https://`).
    #[arg(long)]
    pub acme_directory: Option<String>,
    /// Opt-in HSTS `max-age` in seconds; sent only on CA-trusted native TLS or trusted-proxy
    /// HTTPS requests.
    #[arg(long)]
    pub hsts_max_age_secs: Option<u64>,
    /// How long a peer may make no progress receiving a response, or a WebSocket send may stay
    /// pending, before the server cuts it (`Config.egress_window_secs`; 1 to 3600).
    #[arg(long)]
    pub egress_window_secs: Option<u64>,
    /// Fixes automatic router port forwarding on or off (`Config.port_forward`), overriding the
    /// admin toggle in both directions.
    #[arg(long)]
    pub port_forward: Option<bool>,
    /// Turns the guided onboarding tours on or off for every user (`Config.onboarding_enabled`).
    #[arg(long)]
    pub onboarding_enabled: Option<bool>,
    /// Subpath the whole server is mounted under behind a reverse proxy that does not rewrite
    /// paths (`Config.route_prefix`), e.g. `/ttrpg`.
    #[arg(long)]
    pub route_prefix: Option<String>,
    /// One-shot: regenerate the persisted self-signed certificate and exit. Mutually exclusive
    /// with `--backup-to`, `--restore-from` and any subcommand.
    #[arg(long)]
    pub tls_regenerate: bool,
    /// Subcommand; `None` (the default — no subcommand given) is ordinary serve mode. Every
    /// root flag above parses unconditionally regardless of which variant, if any, this holds.
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

/// Top-level subcommands of the `shadowcat` binary.
///
/// # Examples
///
/// ```
/// use clap::Parser;
/// use shadowcat::config::{Cli, CliCommand};
///
/// let cli = Cli::parse_from(["shadowcat", "audio-monitor", "--port", "0"]);
/// assert!(matches!(cli.command, Some(CliCommand::AudioMonitor(_))));
/// ```
#[derive(clap::Subcommand, Debug)]
pub enum CliCommand {
    /// Serve the localhost audio-session-level WebSocket the ducking module's
    /// `OsMonitorSource` connects to (`crate::audio_monitor::server::run`). Never starts the
    /// main world server; mutually exclusive with `--backup-to`/`--restore-from`.
    AudioMonitor(AudioMonitorArgs),
}

/// Arguments for `shadowcat audio-monitor`.
///
/// # Examples
///
/// ```
/// use clap::Parser;
/// use shadowcat::config::{Cli, CliCommand};
///
/// let cli = Cli::parse_from(["shadowcat", "audio-monitor", "--watch", "discord", "--watch", "teams"]);
/// let Some(CliCommand::AudioMonitor(args)) = cli.command else {
///     unreachable!()
/// };
/// assert_eq!(args.port, 31998); // the built-in default
/// assert_eq!(args.watch, vec!["discord".to_string(), "teams".to_string()]);
/// ```
#[derive(clap::Args, Debug)]
pub struct AudioMonitorArgs {
    /// Localhost port to bind. `0` binds an ephemeral port (printed on start).
    #[arg(long, default_value_t = 31998)]
    pub port: u16,
    /// Allowed `Origin` header values. When none are given the built-in defaults apply
    /// (`http`/`https` on `localhost`/`127.0.0.1` port 30000, `audio_monitor::server::DEFAULT_ALLOWED_ORIGINS`);
    /// a non-empty list replaces them.
    #[arg(long)]
    pub allow_origin: Vec<String>,
    /// Initial watched-process substrings (case-insensitive); default `discord` when empty.
    /// The client's live `watch` frame replaces this list at runtime without a restart.
    #[arg(long)]
    pub watch: Vec<String>,
}

/// Effective server configuration after layering. Precedence (high→low):
/// CLI flag > SHADOWCAT_* env > TOML file > built-in default.
///
/// # Examples
///
/// ```
/// use shadowcat::config::Config;
///
/// let cfg = Config { bind: "0.0.0.0:9000".into(), ..Default::default() };
/// assert_eq!(cfg.bind, "0.0.0.0:9000");
/// assert_eq!(cfg.db, "./shadowcat.db"); // untouched fields keep the default
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen address (`host:port`). Default `127.0.0.1:30000`; non-loopback
    /// binds flip the `auto` setup policy to token-required.
    pub bind: String,
    /// SQLite database file path. Default `./shadowcat.db`; sibling dirs
    /// (`assets/`, `modules/`, `backups/`) resolve relative to it.
    pub db: String,
    /// Headless first-admin bootstrap username; `None` = interactive setup form.
    pub admin_user: Option<String>,
    /// Headless first-admin bootstrap password; `None` = interactive setup form.
    pub admin_password: Option<String>,
    /// `"auto"` | `"off"` | `"required"` | `<explicit token>`.
    pub setup_token: String,
    /// Session-cookie signing key (base64). `None` = generated once and
    /// persisted to the DB `settings` table, then loaded on every later boot —
    /// sessions already survive an ordinary restart unpinned. Set it explicitly
    /// for replicas sharing one key or controlled rotation
    /// (`auth::session::load_or_create_key`).
    pub session_key: Option<String>,
    /// Asset storage root. `None` → sibling `assets/` beside the db file.
    pub assets_dir: Option<String>,
    /// Installed-module discovery root. `None` → sibling `modules/` dir beside the db file.
    pub modules_dir: Option<String>,
    /// Rolling log-file output root (`main::init_tracing`). `None` → sibling `logs/`
    /// dir beside the db file, mirroring `assets_dir`/`modules_dir`'s own convention.
    pub log_dir: Option<String>,
    /// In-server backup output root (`POST /api/admin/backup`). `None` → sibling
    /// `backups/` dir beside the db file.
    pub backups_dir: Option<String>,
    /// Retention: keep the newest N in-app backups. `None` = no count limit. With
    /// `backup_keep_days` also set, a backup survives when EITHER limit keeps it (union), and
    /// the newest backup is never pruned (`backup::catalog::plan_prune`).
    pub backup_keep_last: Option<u32>,
    /// Retention: keep in-app backups younger than N days. `None` = no age limit. Both limits
    /// `None` keeps every backup.
    pub backup_keep_days: Option<u32>,
    /// How long a discarded file survives in the trash: a positive number of days (default 30),
    /// `"never"` (no automatic sweep) or `"disabled"` (every removal is immediate and
    /// unrecoverable). Zero is a boot error, not a spelling of `"disabled"`.
    pub trash_retention: crate::trash::TrashRetention,
    /// Trash storage root. `None` → [`Config::trash_path`]'s default.
    pub trash_dir: Option<String>,
    /// Regular-uploader size cap (bytes). Default 25 MiB.
    pub upload_max_bytes: u64,
    /// Regular-uploader uploads per minute. Default 20.
    pub upload_rate_per_min: u32,
    /// GM/owner size cap; `None` → 2× `upload_max_bytes`.
    pub upload_max_bytes_gm: Option<u64>,
    /// GM/owner uploads per minute; `None` → 2× `upload_rate_per_min`.
    pub upload_rate_per_min_gm: Option<u32>,
    /// Whether a converted upload keeps its original bytes on disk as
    /// `<uuid>.orig` (reconvert + "download original" need it). Default
    /// `true`; `false` trades those for disk. Host-level: the host pays for
    /// the disk, so this is not a per-world setting.
    pub retain_originals: bool,
    /// Per-identity `/api/login` budget (trailing 60s); `None` →
    /// `throttle::LOGIN_PER_MIN_PER_IDENTITY`. Self-hosting operators behind a
    /// NAT/shared-proxy IP, or an automated test harness that logs in as one
    /// identity repeatedly, may need to raise this — never lower it below a
    /// value that still bounds credential stuffing.
    pub login_per_min_per_identity: Option<usize>,
    /// Per-IP `/api/login` budget (trailing 60s); `None` →
    /// `throttle::LOGIN_PER_MIN_PER_IP`.
    pub login_per_min_per_ip: Option<usize>,
    /// Per-account `/api/invites/accept` budget (trailing 60s); `None` →
    /// `throttle::INVITE_PER_MIN_PER_ACCOUNT`.
    pub invite_per_min_per_account: Option<usize>,
    /// Per-IP `/api/invites/accept` budget (trailing 60s); `None` →
    /// `throttle::INVITE_PER_MIN_PER_IP`.
    pub invite_per_min_per_ip: Option<usize>,
    /// Per-account `POST /api/worlds` budget (trailing 60s); `None` →
    /// `throttle::WORLD_CREATE_PER_MIN_PER_ACCOUNT`. Same rationale as
    /// `login_per_min_per_identity`: an automated test harness that creates many
    /// worlds under one seeded identity across a long-running session needs to raise
    /// this — never lower it below a value that still bounds a scripted flood.
    pub world_create_per_min_per_account: Option<usize>,
    /// Exact peer IP addresses trusted to set `X-Forwarded-For` and `X-Forwarded-Proto`
    /// (reverse-proxy deployments only). Listing a proxy ALSO exempts it from the per-peer
    /// connection caps, and so does a loopback peer without being listed
    /// ([`Config::is_cap_exempt`]); only a listed proxy has its forwarded headers believed.
    /// Parsed as IP literals when the configuration is extracted, so an entry that is not an IP
    /// address is a boot error naming the key and the value. Default empty — `ClientIp` never
    /// consults the header and resolves solely from the TCP peer
    /// address. Matched by EXACT address only (an IPv4-mapped IPv6 form and its IPv4 form are the
    /// same address), not CIDR range
    /// (self-hosting operators list every trusted proxy's address explicitly; a typical single
    /// reverse-proxy-on-the-same-host deployment needs one entry, e.g. `"127.0.0.1"`). A request
    /// whose immediate TCP peer is NOT in this list gets its `X-Forwarded-For` header ignored
    /// entirely, regardless of content — this is what prevents an arbitrary internet client from
    /// spoofing its own address by sending the header directly.
    #[serde(deserialize_with = "deserialize_ip_list")]
    pub trusted_proxies: Vec<std::net::IpAddr>,
    /// Declares a reverse proxy fronts `bind` on the same host. Default `false`, never
    /// inferred. `is_exposed()` folds it with `is_loopback_bind()` into the predicate
    /// `setup_token_policy`'s `"auto"` arm and `AppState::resolve_setup_token`'s warning read.
    /// Whether a given request arrived over HTTPS is NOT decided here: it is decided per request
    /// by `http::scheme::request_scheme` (native TLS, or `X-Forwarded-Proto` from a peer in
    /// `trusted_proxies`).
    pub behind_proxy: bool,
    /// Built-in HTTPS mode: `auto` (default; TLS answered on the same port as plain HTTP) or `off`.
    pub tls: TlsMode,
    /// Operator certificate chain (PEM path). Both or neither of `tls_cert`/`tls_key`.
    pub tls_cert: Option<String>,
    /// Operator private key (PEM path).
    pub tls_key: Option<String>,
    /// Extra DNS names / IP literals the self-signed certificate must cover, and (DNS entries)
    /// hostnames published as HTTPS origins.
    pub tls_hostnames: Vec<String>,
    /// Whether the self-signed certificate lists this machine's hostname and its `<hostname>.local`
    /// form. Default `false`: the certificate is presented to every peer before authentication,
    /// and a machine hostname commonly embeds a person's name. Turning it on lets players reach
    /// the server by machine name over HTTPS without a name-mismatch error stacked on the
    /// self-signed trust warning. Changing the value regenerates the persisted certificate at the
    /// next start (which re-shows the browser warning once). Interface IP addresses are always
    /// listed.
    pub tls_advertise_hostname: bool,
    /// TLS state directory (self-signed material, ACME account). `None` → sibling `tls/` beside
    /// the db file.
    pub tls_dir: Option<String>,
    /// Absolute `https://` origin players use to reach this server. Published as the ASCII
    /// serialization of the origin `https_origin_of` extracts (scheme, lowercased host, and a
    /// port unless it is the scheme default); the value must be exactly an origin.
    pub public_url: Option<String>,
    /// DNS names to obtain a certificate for through ACME.
    pub acme_domains: Vec<String>,
    /// PEM file with the private ACME CA's root (or intermediate + root) to trust for ACME only.
    pub acme_ca_root: Option<String>,
    /// ACME account contact e-mail.
    pub acme_email: Option<String>,
    /// Must be `true` to use ACME: confirms the CA's terms of service are accepted.
    pub acme_terms_agreed: bool,
    /// ACME challenge type.
    pub acme_challenge: AcmeChallenge,
    /// ACME directory URL; must be `https://`.
    pub acme_directory: String,
    /// Opt-in HSTS `max-age` (seconds). `None` (default) never sends HSTS.
    pub hsts_max_age_secs: Option<u64>,
    /// How long, in seconds, a peer may make no progress receiving data before the server cuts it
    /// (1 to [`EGRESS_WINDOW_SECS_MAX`]; default [`crate::http::conn_serve::PEER_STALL_TIMEOUT`],
    /// 60). One value: it is the no-progress window of every response and the bound on one pending
    /// WebSocket send ([`Config::egress_window`]), so the two cannot be tuned apart. A reader that
    /// the socket can only feed in lumps larger than one window's worth of its reading rate is cut
    /// while still reading; raise this on extremely slow networks. A connection with nothing in
    /// flight is never cut by it.
    pub egress_window_secs: u64,
    /// Fixes automatic router port forwarding (NAT-PMP, PCP, UPnP) on or off. `None` (default)
    /// leaves it to the admin toggle stored in the database, which is off until an admin turns it
    /// on. When set it wins over the stored toggle in both directions and the admin route refuses
    /// to change it. Enabling opens the server's port to the internet.
    pub port_forward: Option<bool>,
    /// Whether clients offer the guided onboarding tours. Default `true`; `false` suppresses the
    /// offer for every user on this server. Published as `onboarding_enabled` in `/api/config`
    /// (`http::routes::ServerConfig`).
    pub onboarding_enabled: bool,
    /// Subpath every route is served under (`http::router` nests the whole application there),
    /// for a reverse proxy that forwards `https://host/<prefix>/...` without rewriting the path.
    /// `None` (default) mounts at the origin root. `Config::load` normalizes it once through
    /// [`normalize_route_prefix`] (leading slash, no trailing slash); read it through
    /// [`Config::base_path`] only.
    pub route_prefix: Option<String>,
}

/// The least [`Config::egress_window_secs`] accepts.
pub const EGRESS_WINDOW_SECS_MIN: u64 = 1;

/// The most [`Config::egress_window_secs`] accepts: one hour, far above any peer that is merely
/// slow and low enough that a stalled peer cannot pin a connection for a day.
pub const EGRESS_WINDOW_SECS_MAX: u64 = 3600;

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:30000".into(),
            db: "./shadowcat.db".into(),
            admin_user: None,
            admin_password: None,
            setup_token: "auto".into(),
            session_key: None,
            assets_dir: None,
            modules_dir: None,
            log_dir: None,
            backups_dir: None,
            backup_keep_last: None,
            backup_keep_days: None,
            trash_retention: crate::trash::TrashRetention::default(),
            trash_dir: None,
            upload_max_bytes: 25 * 1024 * 1024,
            upload_rate_per_min: 20,
            upload_max_bytes_gm: None,
            upload_rate_per_min_gm: None,
            retain_originals: true,
            login_per_min_per_identity: None,
            login_per_min_per_ip: None,
            invite_per_min_per_account: None,
            invite_per_min_per_ip: None,
            world_create_per_min_per_account: None,
            trusted_proxies: Vec::new(),
            behind_proxy: false,
            tls: TlsMode::Auto,
            tls_cert: None,
            tls_key: None,
            tls_hostnames: Vec::new(),
            tls_advertise_hostname: false,
            tls_dir: None,
            public_url: None,
            acme_domains: Vec::new(),
            acme_ca_root: None,
            acme_email: None,
            acme_terms_agreed: false,
            acme_challenge: AcmeChallenge::TlsAlpn01,
            acme_directory: DEFAULT_ACME_DIRECTORY.into(),
            hsts_max_age_secs: None,
            egress_window_secs: crate::http::conn_serve::PEER_STALL_TIMEOUT.as_secs(),
            port_forward: None,
            onboarding_enabled: true,
            route_prefix: None,
        }
    }
}

/// Normalizes a configured route prefix to the one form the router, the embedded-page
/// substitution and the client all share: a leading slash and no trailing slash. An empty value
/// or a bare `/` means the origin root and yields `None`. Each segment must be non-empty and made
/// of URL-unreserved characters (`A-Z a-z 0-9 - . _ ~`), and may not be `.` or `..`: the value is
/// spliced into the served HTML and handed to the router's `nest`, which rejects route-pattern
/// characters, so anything else is a boot error rather than a silently different mount.
///
/// # Errors
///
/// Returns the reason when a segment is empty, dot-relative or holds a character outside the set.
///
/// # Examples
///
/// ```
/// use shadowcat::config::normalize_route_prefix;
///
/// assert_eq!(normalize_route_prefix("ttrpg").unwrap().as_deref(), Some("/ttrpg"));
/// assert_eq!(normalize_route_prefix("/ttrpg/").unwrap().as_deref(), Some("/ttrpg"));
/// assert_eq!(normalize_route_prefix("/a/b").unwrap().as_deref(), Some("/a/b"));
/// assert_eq!(normalize_route_prefix("/").unwrap(), None);
/// assert!(normalize_route_prefix("/a//b").is_err());
/// assert!(normalize_route_prefix("/a/{id}").is_err());
/// ```
pub fn normalize_route_prefix(raw: &str) -> Result<Option<String>, String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(None);
    }
    for segment in trimmed.split('/') {
        let valid = !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
        if !valid {
            return Err(format!(
                "route_prefix {raw:?} must be slash-separated segments of letters, digits, '-', '.', '_' or '~' (no empty, '.' or '..' segment)"
            ));
        }
    }
    Ok(Some(format!("/{trimmed}")))
}

/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
///
/// # Examples
///
/// ```
/// use shadowcat::config::SetupTokenPolicy;
///
/// let policy = SetupTokenPolicy::Required(Some("secret".to_string()));
/// assert!(matches!(policy, SetupTokenPolicy::Required(Some(_))));
/// assert!(matches!(SetupTokenPolicy::Open, SetupTokenPolicy::Open));
/// ```
#[derive(Debug, Clone)]
pub enum SetupTokenPolicy {
    /// `/api/setup` accepts the first admin without a token.
    Open,
    /// A token gates `/api/setup`; `None` = generate-and-log at boot.
    Required(Option<String>),
}

/// The retired configuration key `behind_proxy` replaced (TOML key, and the lowercase form the
/// `SHADOWCAT_` env provider produces).
const RETIRED_BEHIND_TLS_PROXY_KEY: &str = "behind_tls_proxy";

/// Boot error text for the retired key: names the replacement and why ignoring it is unsafe.
const RETIRED_BEHIND_TLS_PROXY_MESSAGE: &str = "behind_tls_proxy was renamed behind_proxy; it declares that a reverse proxy fronts this bind. Ignoring it would treat a proxied loopback deployment as local-only. Delete the behind_tls_proxy line/variable even if its value is false. If a reverse proxy fronts this server, migrate fully: (1) set behind_proxy = true; (2) list the proxy's address in trusted_proxies, because the server learns the request scheme (X-Forwarded-Proto) and the client address (X-Forwarded-For) only from a listed proxy, and without the entry every request is treated as plain HTTP; (3) make the proxy OVERWRITE X-Forwarded-Proto (and X-Forwarded-For) with its own values rather than pass a client-supplied header through.";

/// A configuration contradiction or malformed TLS/ACME value, decidable from configuration
/// values alone (no file or network I/O). Everything that needs I/O degrades at runtime instead
/// (see `tls::TlsRuntime`).
///
/// # Examples
///
/// ```
/// use shadowcat::config::Config;
///
/// let cfg = Config { tls_cert: Some("cert.pem".into()), ..Default::default() };
/// let err = cfg.validate_tls().unwrap_err();
/// assert!(err.to_string().contains("tls_key"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TlsConfigError {
    /// An operator certificate and ACME domains were both configured.
    #[error(
        "tls_cert/tls_key and acme_domains are mutually exclusive: choose one certificate source"
    )]
    PemAndAcme,
    /// `tls = "off"` together with certificate material.
    #[error("tls = \"off\" contradicts {0}; remove it or set tls = \"auto\"")]
    OffWithMaterial(&'static str),
    /// Exactly one of `tls_cert`/`tls_key` was set.
    #[error("tls_cert and tls_key must be set together; {0} is missing")]
    HalfPemPair(&'static str),
    /// ACME domains without the terms-of-service confirmation.
    #[error(
        "acme_domains requires acme_terms_agreed = true (accepting the CA's terms of service)"
    )]
    AcmeWithoutTerms,
    /// An `acme_domains` entry that is not a plain DNS name.
    #[error("acme_domains entry {0:?} must be a plain DNS name (no IP literal, wildcard, scheme, port or path)")]
    AcmeDomainInvalid(String),
    /// ACME needs somewhere to persist its account and certificate.
    #[error(
        "acme_domains needs a persistent tls_dir: the database is in-memory and tls_dir is unset"
    )]
    AcmeNeedsTlsDir,
    /// A malformed `tls_hostnames` entry.
    #[error("tls_hostnames entry {0:?} must be a DNS name or IP literal (no scheme, port or path; a wildcard only as a leading \"*.\")")]
    HostnameInvalid(String),
    /// `public_url` is not an `https://` origin.
    #[error("public_url {0:?} must be an https:// origin: scheme, host and optional port only (no user information, path, query or fragment)")]
    PublicUrlInvalid(String),
    /// `acme_directory` is not an `https://` URL.
    #[error("acme_directory {0:?} must be an https:// URL")]
    AcmeDirectoryNotHttps(String),
    /// `acme_ca_root` without any `acme_domains`.
    #[error("acme_ca_root only applies together with acme_domains")]
    AcmeCaRootWithoutDomains,
}

/// Whether `entry` is a syntactically valid certificate host entry: an IP literal, or a DNS
/// name of 1-63 character `[A-Za-z0-9-]` labels (no leading/trailing hyphen, at most 253
/// characters). `allow_wildcard` admits a single leading `*.` label. Scheme, port, path, brackets
/// and trailing dots are all rejected by construction.
///
/// # Examples
///
/// ```
/// use shadowcat::config::is_valid_host_entry;
///
/// assert!(is_valid_host_entry("game.example.com", false));
/// assert!(is_valid_host_entry("192.0.2.7", false));
/// assert!(is_valid_host_entry("*.example.com", true));
/// assert!(!is_valid_host_entry("*.example.com", false));
/// assert!(!is_valid_host_entry("https://example.com", true));
/// assert!(!is_valid_host_entry("example.com:443", true));
/// ```
pub fn is_valid_host_entry(entry: &str, allow_wildcard: bool) -> bool {
    if entry.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let name = if allow_wildcard {
        entry.strip_prefix("*.").unwrap_or(entry)
    } else {
        entry
    };
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// Deserializes a list of IP-address strings, failing with the offending entry's text so a boot
/// error names both the key (added by the config layer) and the value.
///
/// # Examples
///
/// ```
/// let mut ok = serde_json::Deserializer::from_str(r#"["127.0.0.1"]"#);
/// let ips = shadowcat::config::deserialize_ip_list(&mut ok).unwrap();
/// assert_eq!(ips, vec!["127.0.0.1".parse::<std::net::IpAddr>().unwrap()]);
/// let mut bad = serde_json::Deserializer::from_str(r#"["nope"]"#);
/// let err = shadowcat::config::deserialize_ip_list(&mut bad).unwrap_err();
/// assert!(err.to_string().contains("nope"));
/// ```
pub fn deserialize_ip_list<'de, D>(deserializer: D) -> Result<Vec<std::net::IpAddr>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<String> = Deserialize::deserialize(deserializer)?;
    raw.into_iter()
        .map(|entry| {
            entry.trim().parse().map_err(|_| {
                serde::de::Error::custom(format!("entry {entry:?} is not an IP address"))
            })
        })
        .collect()
}

/// Whether `raw` parses as an absolute `https://` URL that names a host.
fn is_https_url_with_host(raw: &str) -> bool {
    url::Url::parse(raw)
        .map(|p| p.scheme() == "https" && p.host_str().is_some())
        .unwrap_or(false)
}

/// The `https://` origin `raw` denotes (`scheme://host[:port]`, default port elided), or `None`
/// when `raw` is not an `https://` URL that is exactly an origin: it must name a host and carry no
/// user information, no path other than `/`, no query and no fragment. This is the single
/// definition of a valid `public_url`; [`Config::validate_tls`] rejects what it rejects and the
/// published origin is what it returns, so the two cannot disagree.
///
/// # Examples
///
/// ```
/// use shadowcat::config::https_origin_of;
///
/// assert_eq!(https_origin_of("https://Play.Example/").as_deref(), Some("https://play.example"));
/// assert_eq!(https_origin_of("https://play.example:30000").as_deref(), Some("https://play.example:30000"));
/// assert_eq!(https_origin_of("https://play.example:443").as_deref(), Some("https://play.example"));
/// assert_eq!(https_origin_of("https://u:p@play.example"), None);
/// assert_eq!(https_origin_of("https://play.example/lobby"), None);
/// assert_eq!(https_origin_of("https://play.example/?x=1"), None);
/// assert_eq!(https_origin_of("https://play.example/#f"), None);
/// assert_eq!(https_origin_of("http://play.example"), None);
/// ```
pub fn https_origin_of(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    let exact = url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    exact.then(|| url.origin().ascii_serialization())
}

impl Config {
    /// Layer file + env over defaults via figment, then apply CLI overrides in
    /// code so CLI strictly wins (figment cannot easily skip `None` CLI fields).
    ///
    /// Fails on anything that cannot be read as configuration at all (malformed TOML, a value of
    /// the wrong type, an IP-list entry that is not an IP address, the retired
    /// `behind_tls_proxy` key). It does NOT run [`Config::validate_tls`]: the cross-field TLS
    /// checks belong to the entry points that actually use TLS settings (serving and
    /// `--tls-regenerate`), so a database backup or restore is never blocked by an unrelated TLS
    /// mistake in the same configuration file.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use shadowcat::config::{Cli, Config};
    ///
    /// // Reads shadowcat.toml (if present) + SHADOWCAT_* env, then applies CLI wins.
    /// let cfg = Config::load(Cli::default()).expect("config layering");
    /// assert!(!cfg.bind.is_empty());
    /// ```
    // Boxed: figment::Error is third-party and large by value, so returning it inline
    // would widen every Result in the boot path to that error's size.
    pub fn load(cli: Cli) -> Result<Self, Box<figment::Error>> {
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| "shadowcat.toml".into());
        let figment = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&config_path)) // missing file is ignored
            .merge(Env::prefixed("SHADOWCAT_"));
        // The retired key is detected in every file/env layer BEFORE extraction: `Config` no
        // longer names it, so a stale value would otherwise be dropped silently and a proxied
        // loopback deployment would run as local-only.
        if figment.contains(RETIRED_BEHIND_TLS_PROXY_KEY) {
            return Err(Box::new(figment::Error::from(
                RETIRED_BEHIND_TLS_PROXY_MESSAGE,
            )));
        }
        let mut cfg: Config = figment.extract()?;

        if let Some(v) = cli.bind {
            cfg.bind = v;
        }
        if let Some(v) = cli.db {
            cfg.db = v;
        }
        if let Some(v) = cli.admin_user {
            cfg.admin_user = Some(v);
        }
        if let Some(v) = cli.admin_password {
            cfg.admin_password = Some(v);
        }
        if let Some(v) = cli.setup_token {
            cfg.setup_token = v;
        }
        if let Some(v) = cli.session_key {
            cfg.session_key = Some(v);
        }
        if let Some(v) = cli.retain_originals {
            cfg.retain_originals = v;
        }
        if let Some(v) = cli.assets_dir {
            cfg.assets_dir = Some(v);
        }
        if let Some(v) = cli.modules_dir {
            cfg.modules_dir = Some(v);
        }
        if let Some(v) = cli.log_dir {
            cfg.log_dir = Some(v);
        }
        if let Some(v) = cli.backups_dir {
            cfg.backups_dir = Some(v);
        }
        if let Some(v) = cli.backup_keep_last {
            cfg.backup_keep_last = Some(v);
        }
        if let Some(v) = cli.backup_keep_days {
            cfg.backup_keep_days = Some(v);
        }
        if let Some(v) = cli.trash_retention {
            cfg.trash_retention = v.parse().map_err(|e: crate::trash::TrashRetentionError| {
                Box::new(figment::Error::from(e.to_string()))
            })?;
        }
        if let Some(v) = cli.trash_dir {
            cfg.trash_dir = Some(v);
        }
        if let Some(v) = cli.behind_proxy {
            cfg.behind_proxy = v;
        }
        if let Some(v) = cli.tls {
            cfg.tls = v;
        }
        if let Some(v) = cli.tls_cert {
            cfg.tls_cert = Some(v);
        }
        if let Some(v) = cli.tls_key {
            cfg.tls_key = Some(v);
        }
        if let Some(v) = cli.tls_hostnames {
            cfg.tls_hostnames = v;
        }
        if let Some(v) = cli.tls_advertise_hostname {
            cfg.tls_advertise_hostname = v;
        }
        if let Some(v) = cli.tls_dir {
            cfg.tls_dir = Some(v);
        }
        if let Some(v) = cli.public_url {
            cfg.public_url = Some(v);
        }
        if let Some(v) = cli.acme_domains {
            cfg.acme_domains = v;
        }
        if let Some(v) = cli.acme_ca_root {
            cfg.acme_ca_root = Some(v);
        }
        if let Some(v) = cli.acme_email {
            cfg.acme_email = Some(v);
        }
        if let Some(v) = cli.acme_terms_agreed {
            cfg.acme_terms_agreed = v;
        }
        if let Some(v) = cli.acme_challenge {
            cfg.acme_challenge = v;
        }
        if let Some(v) = cli.acme_directory {
            cfg.acme_directory = v;
        }
        if let Some(v) = cli.hsts_max_age_secs {
            cfg.hsts_max_age_secs = Some(v);
        }
        if let Some(v) = cli.egress_window_secs {
            cfg.egress_window_secs = v;
        }
        if let Some(v) = cli.port_forward {
            cfg.port_forward = Some(v);
        }
        if let Some(v) = cli.onboarding_enabled {
            cfg.onboarding_enabled = v;
        }
        if let Some(v) = cli.route_prefix {
            cfg.route_prefix = Some(v);
        }
        // Normalized after every layer has applied, so a prefix from the CLI, the environment or
        // the TOML file reaches every reader in the same form.
        if let Some(raw) = cfg.route_prefix.take() {
            cfg.route_prefix = normalize_route_prefix(&raw).map_err(figment::Error::from)?;
        }
        if !(EGRESS_WINDOW_SECS_MIN..=EGRESS_WINDOW_SECS_MAX).contains(&cfg.egress_window_secs) {
            return Err(Box::new(figment::Error::from(format!(
                "egress_window_secs must be {EGRESS_WINDOW_SECS_MIN} to {EGRESS_WINDOW_SECS_MAX}, got {}",
                cfg.egress_window_secs
            ))));
        }
        cfg.validate_trash_placement()?;
        Ok(cfg)
    }

    /// Rejects a `trash_dir` that is the same as, or nested inside, `assets_dir`, `modules_dir`,
    /// `backups_dir` or `log_dir`, and any of those nested inside it. A trash inside the asset
    /// root would be walked by the asset routes and copied by every backup; one inside the
    /// backups root would be captured by the next backup. The error names both paths.
    ///
    /// The backups and log roots are skipped when the database is in-memory and no override names
    /// them (they have no location to collide with). Paths are compared component-wise after
    /// resolving a relative one against the current directory, without touching the filesystem,
    /// since none of these directories need exist at load time.
    ///
    /// # Examples
    ///
    /// ```text
    /// Config::default().validate_trash_placement()  // Ok: trash/ and assets/ are siblings
    /// // trash_dir = "data/assets/trash", assets_dir = "data/assets" -> Err naming both
    /// ```
    fn validate_trash_placement(&self) -> Result<(), Box<figment::Error>> {
        let trash = self.trash_path();
        let backups = crate::db::database_file(&self.db)
            .ok()
            .flatten()
            .map(|file| self.backups_path_for(&file))
            .or_else(|| self.backups_dir.as_ref().map(std::path::PathBuf::from));
        let others = [
            ("assets_dir", Some(self.assets_path())),
            ("modules_dir", Some(self.modules_path())),
            ("backups_dir", backups),
            ("log_dir", self.logs_path()),
        ];
        for (label, other) in others {
            let Some(other) = other else { continue };
            if Self::paths_nest(&trash, &other) {
                return Err(Box::new(figment::Error::from(format!(
                    "trash_dir ({}) must not be the same as, or nested inside, {label} ({}), and {label} must not be nested inside trash_dir: a trash sharing a tree with a served or backed-up directory would be walked or backed up",
                    trash.display(),
                    other.display(),
                ))));
            }
        }
        Ok(())
    }

    /// Whether `a` and `b` name the same directory or one is an ancestor of the other, compared
    /// component-wise (so `assets-extra` is not under `assets`) after resolving a relative path
    /// against the current directory.
    ///
    /// # Examples
    ///
    /// ```text
    /// paths_nest("/d/trash", "/d/assets") == false
    /// paths_nest("/d/assets/trash", "/d/assets") == true
    /// ```
    fn paths_nest(a: &std::path::Path, b: &std::path::Path) -> bool {
        let absolute = |p: &std::path::Path| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(p)
            }
        };
        let (a, b) = (absolute(a), absolute(b));
        a.starts_with(&b) || b.starts_with(&a)
    }

    /// Resolve the trash storage root: explicit `trash_dir`, else a sibling `trash/` beside the db
    /// file via `db_sibling_dir`, else a sibling of [`Config::assets_path`]. The fallback serves an
    /// in-memory database with an explicit `assets_dir`: the trash then lands on the volume the
    /// assets live on, so discarding an asset is a rename and not a cross-device copy. A bare
    /// single-component `assets_dir` has an empty parent, which resolves to `.`.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.trash_path(), std::path::Path::new("data").join("trash"));
    /// let memory = shadowcat::config::Config {
    ///     db: "sqlite::memory:".into(),
    ///     assets_dir: Some("vol/assets".into()),
    ///     ..Default::default()
    /// };
    /// assert_eq!(memory.trash_path(), std::path::Path::new("vol").join("trash"));
    /// ```
    pub fn trash_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.trash_dir {
            return std::path::PathBuf::from(dir);
        }
        self.db_sibling_dir("trash").unwrap_or_else(|| {
            self.assets_path()
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("trash")
        })
    }

    /// The route prefix as a string slice: `""` at the origin root, otherwise the normalized
    /// `/prefix`. The one reader of [`Config::route_prefix`]; `http::router` nests under it and
    /// `http::embed::static_handler` strips and publishes it.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::Config;
    ///
    /// assert_eq!(Config::default().base_path(), "");
    /// let cfg = Config { route_prefix: Some("/ttrpg".into()), ..Config::default() };
    /// assert_eq!(cfg.base_path(), "/ttrpg");
    /// ```
    pub fn base_path(&self) -> &str {
        self.route_prefix.as_deref().unwrap_or("")
    }

    /// The no-progress window every stalled-peer bound uses: [`Config::egress_window_secs`] as a
    /// duration. The one reader of that key; `ConnLimits::for_config` and
    /// `BoundedSink::for_config` both call it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::config::Config;
    ///
    /// let cfg = Config { egress_window_secs: 90, ..Config::default() };
    /// assert_eq!(cfg.egress_window(), Duration::from_secs(90));
    /// ```
    pub fn egress_window(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.egress_window_secs)
    }

    /// Resolves a directory named `name` as a sibling of the on-disk database
    /// file `crate::db::database_file` (the single decider for "which file on
    /// disk is `self.db`" -- see its own doc) resolves `self.db` to, or `None`
    /// when `self.db` names an in-memory database or fails to parse. An
    /// unparseable `self.db` yields the same `None` fallback as an in-memory
    /// one here, because `SqliteRepository::connect` already rejects such a
    /// string at server startup, before any caller of this resolver ever
    /// runs. Shared by every sibling-directory resolver below so the "what
    /// counts as a filesystem-backed db" decision is made in exactly one
    /// place rather than re-derived per resolver.
    ///
    /// # Examples
    ///
    /// ```text
    /// db_sibling_dir("backups") == None  // in-memory or malformed self.db
    /// ```
    fn db_sibling_dir(&self, name: &str) -> Option<std::path::PathBuf> {
        let file = crate::db::database_file(&self.db).ok().flatten()?;
        Some(Self::sibling_of(&file, name))
    }

    /// The directory named `name` beside the database file `db_file`. A bare
    /// file name (no parent component) resolves against the current
    /// directory: `Path::parent` returns `Some("")` for it, which is filtered
    /// to `.` so the result is never an empty-rooted path. The one place the
    /// sibling layout is spelled — `db_sibling_dir` and `backups_path_for`
    /// both derive through it.
    ///
    /// # Examples
    ///
    /// ```text
    /// sibling_of("/data/shadowcat.db", "logs") == "/data/logs"
    /// sibling_of("shadowcat.db", "logs")       == "./logs"
    /// ```
    fn sibling_of(db_file: &std::path::Path, name: &str) -> std::path::PathBuf {
        db_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(name)
    }

    /// Resolve the asset storage root: explicit `assets_dir`, else a sibling
    /// `assets/` directory beside the db file (built via std::path, #2), via
    /// `db_sibling_dir`. Falls back to the current directory when the db has
    /// no on-disk location (in-memory, or unparseable) and no `assets_dir`
    /// override was given -- asset storage is required infrastructure
    /// regardless of where (or whether) the database lives on disk.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.assets_path(), std::path::Path::new("data").join("assets"));
    /// ```
    pub fn assets_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.assets_dir {
            return std::path::PathBuf::from(dir);
        }
        self.db_sibling_dir("assets")
            .unwrap_or_else(|| std::path::PathBuf::from(".").join("assets"))
    }

    /// Resolve the installed-module discovery root: explicit `modules_dir`, else a
    /// sibling `modules/` directory beside the db file (built via std::path, #2),
    /// via `db_sibling_dir`, falling back to the current directory on the same terms
    /// as `assets_path`. Unlike `assets_path`, nothing writes here server-side
    /// (install is a manual filesystem extract — no server-side installer exists) —
    /// the directory need not exist; a missing dir scans as "no modules installed"
    /// (see `modules::scan_installed_modules`).
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     modules_dir: Some("staged-modules".into()),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.modules_path(), std::path::PathBuf::from("staged-modules"));
    /// ```
    pub fn modules_path(&self) -> std::path::PathBuf {
        if let Some(dir) = &self.modules_dir {
            return std::path::PathBuf::from(dir);
        }
        self.db_sibling_dir("modules")
            .unwrap_or_else(|| std::path::PathBuf::from(".").join("modules"))
    }

    /// Resolve the rolling log-file output root: explicit `log_dir` always wins (even over an
    /// in-memory `db`, since an operator who names a directory explicitly clearly wants file
    /// logging); otherwise `None` when `db_sibling_dir` cannot place a directory beside the db
    /// file — the db is in-memory (either `:memory:` spelling, or a `mode=memory` query
    /// parameter — see `crate::db::database_file`) or its connection string fails to parse;
    /// otherwise a sibling `logs/` directory beside the db file (built via std::path, #2), the
    /// same sibling-of-the-db-file convention `assets_path`/`modules_path` use. `None` means file
    /// logging is off entirely (`init_tracing` falls back to stdout-only) — never a directory
    /// this function invents on the caller's behalf for a database it cannot locate on disk.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.logs_path(), Some(std::path::Path::new("data").join("logs")));
    ///
    /// let memory_cfg = shadowcat::config::Config {
    ///     db: "sqlite::memory:".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(memory_cfg.logs_path(), None);
    /// ```
    pub fn logs_path(&self) -> Option<std::path::PathBuf> {
        if let Some(dir) = &self.log_dir {
            return Some(std::path::PathBuf::from(dir));
        }
        self.db_sibling_dir("logs")
    }

    /// Resolve the backup output root for the on-disk database file
    /// `db_file`: explicit `backups_dir`, else a sibling `backups/` directory
    /// beside `db_file` (built via std::path, #2). Takes the already-resolved
    /// file rather than re-deriving it from `self.db`, so the caller decides
    /// once — through `crate::db::database_file` — whether a backup is
    /// possible at all (an in-memory `db` has no file to back up) and this
    /// resolver only places the output.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default();
    /// let db_file = std::path::Path::new("data").join("shadowcat.db");
    /// assert_eq!(cfg.backups_path_for(&db_file), std::path::Path::new("data").join("backups"));
    /// cfg.backups_dir = Some("elsewhere".into());
    /// assert_eq!(cfg.backups_path_for(&db_file), std::path::PathBuf::from("elsewhere"));
    /// ```
    pub fn backups_path_for(&self, db_file: &std::path::Path) -> std::path::PathBuf {
        match &self.backups_dir {
            Some(dir) => std::path::PathBuf::from(dir),
            None => Self::sibling_of(db_file, "backups"),
        }
    }

    /// Role-tiered upload size cap (GM defaults to 2× the regular value).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::WorldRole;
    ///
    /// let cfg = shadowcat::config::Config::default();
    /// assert_eq!(cfg.effective_max_bytes(WorldRole::Player), 25 * 1024 * 1024);
    /// assert_eq!(cfg.effective_max_bytes(WorldRole::Gm), 2 * cfg.effective_max_bytes(WorldRole::Player));
    /// ```
    pub fn effective_max_bytes(&self, role: crate::data::document::WorldRole) -> u64 {
        match role {
            crate::data::document::WorldRole::Gm => self
                .upload_max_bytes_gm
                .unwrap_or(self.upload_max_bytes.saturating_mul(2)),
            _ => self.upload_max_bytes,
        }
    }

    /// Role-tiered uploads-per-minute (GM defaults to 2× the regular value).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::document::WorldRole;
    ///
    /// let cfg = shadowcat::config::Config::default(); // 20/min player rate
    /// assert_eq!(cfg.effective_rate_per_min(WorldRole::Gm), 40);
    /// ```
    pub fn effective_rate_per_min(&self, role: crate::data::document::WorldRole) -> u32 {
        match role {
            crate::data::document::WorldRole::Gm => self
                .upload_rate_per_min_gm
                .unwrap_or(self.upload_rate_per_min.saturating_mul(2)),
            _ => self.upload_rate_per_min,
        }
    }

    /// True when the bind host resolves to a loopback address. `0.0.0.0` /
    /// non-loopback hosts are treated as exposed. On parse failure, default to
    /// the safe answer (not loopback) so the token is required.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default(); // 127.0.0.1:30000
    /// assert!(cfg.is_loopback_bind());
    /// cfg.bind = "0.0.0.0:30000".into();
    /// assert!(!cfg.is_loopback_bind());
    /// ```
    pub fn is_loopback_bind(&self) -> bool {
        self.bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr: SocketAddr| addr.ip().is_loopback())
            .unwrap_or(false)
    }

    /// True when this deployment is reachable from beyond the local host: a non-loopback
    /// bind, OR a loopback bind explicitly declared `behind_proxy` (a reverse proxy on the same
    /// host forwards internet traffic to it). This is the predicate every "is this deployment
    /// exposed" security decision reads: `setup_token_policy`'s `"auto"` arm and
    /// `AppState::resolve_setup_token`'s warning call this rather than `is_loopback_bind()`
    /// directly, so those cannot drift out of agreement with each other on this input. Which
    /// scheme a request used is a separate, per-request question answered only by
    /// `http::scheme::request_scheme`.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut cfg = shadowcat::config::Config::default(); // loopback, no proxy
    /// assert!(!cfg.is_exposed());
    /// cfg.behind_proxy = true;
    /// assert!(cfg.is_exposed());
    /// ```
    pub fn is_exposed(&self) -> bool {
        !self.is_loopback_bind() || self.behind_proxy
    }

    /// Resolve the TLS state directory: explicit `tls_dir`, else a sibling `tls/` directory beside
    /// the db file via `db_sibling_dir`. `None` when neither exists (in-memory or unparseable db
    /// with no `tls_dir`): key material then lives only in process memory.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config {
    ///     db: "data/shadowcat.db".into(),
    ///     ..Default::default()
    /// };
    /// assert_eq!(cfg.tls_path(), Some(std::path::Path::new("data").join("tls")));
    /// let memory = shadowcat::config::Config { db: "sqlite::memory:".into(), ..Default::default() };
    /// assert_eq!(memory.tls_path(), None);
    /// ```
    pub fn tls_path(&self) -> Option<std::path::PathBuf> {
        if let Some(dir) = &self.tls_dir {
            return Some(std::path::PathBuf::from(dir));
        }
        self.db_sibling_dir("tls")
    }

    /// Rejects configuration that is contradictory or malformed in a way decidable from the
    /// VALUES alone. Called by every entry point that uses TLS settings (`main`'s serve and
    /// `--tls-regenerate` paths, `test_server`), never by [`Config::load`]. Boundary rule: no
    /// file or network I/O happens here, so anything that needs I/O (an unreadable PEM, an
    /// unreachable ACME directory, an unwritable `tls_dir`) is left to degrade at runtime
    /// without taking plain HTTP down.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::Config;
    ///
    /// assert!(Config::default().validate_tls().is_ok());
    /// let bad = Config { public_url: Some("http://example.com".into()), ..Default::default() };
    /// assert!(bad.validate_tls().is_err());
    /// ```
    pub fn validate_tls(&self) -> Result<(), TlsConfigError> {
        let has_pem = self.tls_cert.is_some() || self.tls_key.is_some();
        let has_acme = !self.acme_domains.is_empty();
        if has_pem && has_acme {
            return Err(TlsConfigError::PemAndAcme);
        }
        if self.tls == TlsMode::Off {
            if has_pem {
                return Err(TlsConfigError::OffWithMaterial("tls_cert/tls_key"));
            }
            if has_acme {
                return Err(TlsConfigError::OffWithMaterial("acme_domains"));
            }
        }
        match (&self.tls_cert, &self.tls_key) {
            (Some(_), None) => return Err(TlsConfigError::HalfPemPair("tls_key")),
            (None, Some(_)) => return Err(TlsConfigError::HalfPemPair("tls_cert")),
            _ => {}
        }
        if has_acme {
            if !self.acme_terms_agreed {
                return Err(TlsConfigError::AcmeWithoutTerms);
            }
            for d in &self.acme_domains {
                if d.parse::<std::net::IpAddr>().is_ok()
                    || d.starts_with("*.")
                    || !is_valid_host_entry(d, false)
                {
                    return Err(TlsConfigError::AcmeDomainInvalid(d.clone()));
                }
            }
            if self.tls_path().is_none() {
                return Err(TlsConfigError::AcmeNeedsTlsDir);
            }
        } else if self.acme_ca_root.is_some() {
            return Err(TlsConfigError::AcmeCaRootWithoutDomains);
        }
        for h in &self.tls_hostnames {
            if !is_valid_host_entry(h, true) {
                return Err(TlsConfigError::HostnameInvalid(h.clone()));
            }
        }
        if let Some(u) = &self.public_url {
            if https_origin_of(u).is_none() {
                return Err(TlsConfigError::PublicUrlInvalid(u.clone()));
            }
        }
        if !is_https_url_with_host(&self.acme_directory) {
            return Err(TlsConfigError::AcmeDirectoryNotHttps(
                self.acme_directory.clone(),
            ));
        }
        Ok(())
    }

    /// Operator-facing warnings decidable from configuration alone, logged once at boot.
    ///
    /// # Examples
    ///
    /// ```
    /// let cfg = shadowcat::config::Config { behind_proxy: true, ..Default::default() };
    /// assert_eq!(cfg.startup_warnings().len(), 1);
    /// assert!(shadowcat::config::Config::default().startup_warnings().is_empty());
    /// ```
    pub fn startup_warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.behind_proxy && self.trusted_proxies.is_empty() {
            out.push(
                "behind_proxy is set but trusted_proxies is empty: the request scheme and client address cannot be learned from X-Forwarded-Proto / X-Forwarded-For, so every request is treated as plain HTTP from the proxy's own address. List the proxy's address in trusted_proxies."
                    .to_string(),
            );
        }
        out
    }

    /// Resolve the `setup_token` string into the effective policy: `off` →
    /// open, `required` → token generated at boot, `auto` → derived from
    /// `is_exposed()` (not exposed = open, exposed = required), anything else = that
    /// literal token.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::{Config, SetupTokenPolicy};
    ///
    /// let cfg = Config::default(); // setup_token "auto" + loopback bind
    /// assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
    /// ```
    pub fn setup_token_policy(&self) -> SetupTokenPolicy {
        match self.setup_token.as_str() {
            "off" => SetupTokenPolicy::Open,
            "required" => SetupTokenPolicy::Required(None),
            "auto" => {
                if self.is_exposed() {
                    SetupTokenPolicy::Required(None)
                } else {
                    SetupTokenPolicy::Open
                }
            }
            explicit => SetupTokenPolicy::Required(Some(explicit.to_string())),
        }
    }

    /// Whether `ip` is a configured trusted proxy (`trusted_proxies`), matched by exact address
    /// after both sides are reduced to their canonical form ([`std::net::IpAddr::to_canonical`]),
    /// so a dual-stack socket's `::ffff:192.0.2.1` peer matches a configured `192.0.2.1` and the
    /// reverse. This is the forwarded-header trust decision: the scheme decider and `ClientIp`
    /// believe `X-Forwarded-Proto` and `X-Forwarded-For` only from a peer for which it is true.
    /// It is NOT the connection-cap exemption ([`Config::is_cap_exempt`]), and it has no loopback
    /// arm: widening it would let any local process assert the client address and scheme.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::IpAddr;
    /// let cfg = shadowcat::config::Config {
    ///     trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
    ///     ..Default::default()
    /// };
    /// assert!(cfg.is_trusted_proxy("127.0.0.1".parse::<IpAddr>().unwrap()));
    /// assert!(cfg.is_trusted_proxy("::ffff:127.0.0.1".parse::<IpAddr>().unwrap()));
    /// assert!(!cfg.is_trusted_proxy("8.8.8.8".parse::<IpAddr>().unwrap()));
    /// ```
    pub fn is_trusted_proxy(&self, ip: std::net::IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.trusted_proxies
            .iter()
            .any(|configured| configured.to_canonical() == ip)
    }

    /// Whether `ip` is exempt from the PER-PEER connection caps, both the pre-hand-off admission
    /// cap and the live-connection cap (the global caps still bind it): a configured trusted proxy
    /// ([`Config::is_trusted_proxy`]) or a loopback peer. Every player behind a proxy shares its
    /// address, and a tunnel or proxy on the same host (cloudflared, tailscale, ngrok, nginx)
    /// connects from loopback, so the per-address count says nothing about how many players a
    /// connection from it carries. A shared host with untrusted local users is bounded by the
    /// global caps; there is no per-host switch. `to_canonical` reduces an IPv4-mapped IPv6
    /// address, so `::ffff:127.0.0.1` is loopback too.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::net::IpAddr;
    /// let cfg = shadowcat::config::Config {
    ///     trusted_proxies: vec!["192.0.2.50".parse().unwrap()],
    ///     ..Default::default()
    /// };
    /// assert!(cfg.is_cap_exempt("127.0.0.2".parse::<IpAddr>().unwrap()));
    /// assert!(cfg.is_cap_exempt("::1".parse::<IpAddr>().unwrap()));
    /// assert!(cfg.is_cap_exempt("192.0.2.50".parse::<IpAddr>().unwrap()));
    /// assert!(!cfg.is_cap_exempt("192.0.2.1".parse::<IpAddr>().unwrap()));
    /// // Exemption from the caps never makes a loopback peer a forwarded-header authority.
    /// assert!(!cfg.is_trusted_proxy("127.0.0.1".parse::<IpAddr>().unwrap()));
    /// ```
    pub fn is_cap_exempt(&self, ip: std::net::IpAddr) -> bool {
        self.is_trusted_proxy(ip) || ip.to_canonical().is_loopback()
    }
}

#[cfg(test)]
mod tests;
