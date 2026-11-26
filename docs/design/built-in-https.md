# Built-in HTTPS

Design of the server's built-in TLS: one port serving both `https://` and plain `http://`, the
certificate sources, the per-request scheme decision that drives cookies and HSTS, and the client
behaviour that follows. Operator-facing usage is in the hosting guide
(`docs/site/guides/hosting.md`); the transport-security invariants are in `ARCHITECTURE.md` section 2.

**Status.** Built: the sniffing listener, certificate sources (operator PEM, persisted self-signed,
in-memory fallback), the degradation ladder, SNI selection with the swap cell, the scheme decider,
per-scheme session cookies, HSTS, WebSocket over HTTP/1.1 and HTTP/2, the status routes, the
client changes. **Not built: ACME issuance** (section 6). The ACME configuration keys are validated
and a configured `acme_domains` degrades to the self-signed certificate with an explicit reason;
the resolver's challenge slot and the `acme-tls/1` selection arm exist so issuance slots in
without reshaping any of it. Remaining ACME work is tracked in `PLAN.md`.

Binding directives: (1) plain-HTTP play from ANY address (LAN and internet port-forwarding) is
required — never refuse or degrade login over plain HTTP; (2) Shadowcat serves HTTPS by default, no
external proxy needed, and stronger options stay available; (3) new dependencies are acceptable when
OSI-licensed, preferring pure-Rust rustls; (4) cross-platform from day one.

## 1. Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Listener | ONE port; the first byte of each connection decides: `0x16` = TLS, else plain HTTP. Both `http://host:port` and `https://host:port` serve the full app. No redirect, ever. |
| D2 | Default certificate | Persistent self-signed leaf under the `tls` directory beside the database, TLS ON by default (`tls = "auto"`). Regenerated only on expiry/near-expiry, an unparsable file, or a missing explicitly configured `tls_hostnames` entry. |
| D3 | Sources | Operator PEM and self-signed are built; ACME (TLS-ALPN-01 and HTTP-01) is designed and not built. Contradictory configuration is a boot error; everything else that can fail at runtime DEGRADES (section 3.5) — plain HTTP is never taken down by a TLS failure. |
| D4 | Per-request `Secure` | ONE decider, `http::scheme::request_scheme`; the session layer is built twice and dispatched per request; `Secure`/`HttpOnly`/`SameSite` are enforced on every `__Host-` Set-Cookie including removal cookies (section 4). |
| D5 | Cookie names | `sc_sid` (plain) and `__Host-sc_sid` (TLS). The previous name was tower-sessions' default `id`, so the rename logs every user out once. |
| D6 | HSTS | Off by default; opt-in `hsts_max_age_secs`; sent only when the CLIENT-FACING scheme is TLS and, for native TLS, only when the certificate selected for that handshake is the CA-trusted primary (section 5). |
| D7 | Proxy flag | `behind_tls_proxy` became `behind_proxy`; the retired key is DETECTED in env/TOML and fails boot with an actionable message (no silent security downgrade); a warning when `behind_proxy` is set with empty `trusted_proxies`. |
| D8 | HTTP/2 | ALPN offers `h2` and `http/1.1`; `/ws` is routed for GET and CONNECT (RFC 8441). |
| D9 | Crypto provider | ring only, always passed explicitly; `pnpm lint:crypto-backend` fails if a second TLS backend (its `FORBIDDEN_CRATES`) appears in `Cargo.lock`. |
| D10 | ACME HTTP client (when built) | instant-acme without its `hyper-rustls` feature and a small `HttpClient` over the existing `reqwest`; no platform verifier. |
| D11 | Client | `tls` block in `GET /api/config`, dismissible per-origin HTTPS notice, WebSocket-failure hint, microphone message, audio-monitor `https://` origins. |

## 2. Invariants

1. The request scheme is decided in ONE place (`request_scheme`, over `classify`); cookie flavour,
   HSTS, and any future absolute-URL builder read it.
2. A TLS-side failure of any kind (certificate, ACME, handshake, key file) never prevents plain HTTP
   from serving.
3. Every rustls configuration names its `CryptoProvider` explicitly (`tls::ring_provider`); the
   process never relies on rustls feature-based auto-selection, and `install_default_provider` is
   called once at start so third-party rustls users cannot hit the ambiguity panic either.
4. The certificate presented on a connection, its recorded `ConnMeta.cert`, and the HSTS decision
   derive from ONE `Snapshot::select` call on ONE immutable snapshot (section 3.3).
5. `http::scheme::into_service` is the only place `into_make_service_with_connect_info::<ConnMeta>()`
   is spelled; production, the `test_server` binary and every real-socket test harness build their
   service through it, so `ConnectInfo<ConnMeta>`, `ClientIp`, the scheme decider and the HSTS gate
   have one shape regardless of listener type.

## 3. Certificate sources

### 3.1 Self-signed default — what players see

Each player's browser shows a full-page "not private" interstitial once per browser per certificate
on `https://<host>:<port>`, and again when the certificate changes. Clicking through yields
encryption without server authentication. This is why plain HTTP stays first-class. Startup prints:
trusted source, the published `https://` origins first; self-signed, both URLs, `https://` labelled
"encrypted, browser warns once", `http://` labelled "unencrypted, no warning". The certificate
fingerprint appears ONLY in the log and the admin-only `GET /api/admin/tls`; it is never presented
in player UI as a verification (a fingerprint fetched over the channel being verified proves
nothing; only the out-of-band log/admin copy is meaningful).

### 3.2 Generation and reuse policy (`tls::selfsigned`)

rcgen's `CertificateParams::default()` is unacceptable for a served leaf (validity 1975–4096, no key
usages, CN "rcgen self signed cert"), so generation sets explicitly:

- `not_before = now - 1 day` (clock-skew tolerance), `not_after = now + 397 days`;
- `key_usages = [DigitalSignature]`, `extended_key_usages = [ServerAuth]`, `is_ca = IsCa::ExplicitNoCa`;
- CN `Shadowcat server`, key ECDSA P-256 (ring backend).

SANs: `localhost`, `127.0.0.1`, `::1`, every `tls_hostnames` entry, and the
ELIGIBLE interface addresses at generation time. Eligible = IPv4 private/global and IPv6
global/ULA; excluded: loopback (already present), link-local, unspecified/multicast, and interfaces
whose name matches the virtual-adapter list (`docker*`, `br-*`, `veth*`, `virbr*`, `vmnet*`,
`vboxnet*`, Windows `vEthernet*`). The list is a heuristic proven only by a classifier table test.

Reuse rule at boot and in a daily check (`SELFSIGNED_CHECK_SECS`): keep the stored certificate unless
(a) `not_after` is within 30 days, (b) it does not parse, (c) a configured `tls_hostnames` entry
is missing from its SANs, or (d) it disagrees with the machine-hostname opt-in (below). A NEW interface address does NOT trigger regeneration (VPN/WSL/docker/DHCP
churn would re-warn every player); drift is logged once at INFO. The operator forces regeneration
with `shadowcat --tls-regenerate` (one-shot: regenerate, print the fingerprint, exit; mutually
exclusive with `--backup-to`, `--restore-from` and any subcommand).

Machine hostname (PII): the certificate is presented to every peer that completes a handshake,
before any authentication, and machine hostnames commonly embed a person's name. The hostname and its
`<hostname>.local` form therefore enter the SANs ONLY when `tls_advertise_hostname = true` (default
`false`); the opt-in governs the LAN and public certificates (the host-machine certificate never lists a hostname). The hostname is detected regardless (`SanSources.machine_hostname`) so a
certificate persisted by an earlier run that listed it is recognised: it is regenerated when the
opt-in is off and the hostname is present (unless the operator also named it in `tls_hostnames`,
which is theirs), and regenerated once when the opt-in is turned on and the hostname is absent. A
hostname that fails host validation (non-ASCII, IP literal, `localhost`) is neither listed nor
required, so it can never cause a regeneration loop. `regen_reason` and `SanSet::add_entry`
normalise entries through one function (`normalise_entry`).

Three certificates, chosen by peer (privacy rule). The machine's interface addresses describe the
host's LAN; a player on the internet, or a tunnel on the host machine that fronts one, has no use
for them and must not read them from the handshake (personal-information security outranks
convenience). So three self-signed certificates exist:

- the LAN certificate, above: `localhost`, loopback, `tls_hostnames` and the eligible interface
  addresses. A peer whose address is private (RFC 1918), link-local or IPv6
  unique-local/link-local gets it (`PeerScope::Lan`), unless that peer is a listed
  `trusted_proxies` entry (`Config::is_trusted_proxy`): such a peer fronts players the server
  cannot see, so it gets the public certificate;
- the host-machine certificate (`selfsigned-host-cert.pem` / `selfsigned-host-key.pem`):
  `localhost`, `127.0.0.1` and `::1` ONLY: no interface address, no `tls_hostnames` entry, no
  machine hostname, no external address. A loopback peer that is not a listed trusted proxy is the
  host machine's own browser (`PeerScope::Host`) and gets it. The same peer can also be a tunnel
  agent on the host machine; the tunnel then shows its players this certificate, which names
  nothing of the GM's network. A tunnel the operator lists in `trusted_proxies` gets the public
  certificate instead;
- the public certificate (`selfsigned-public-cert.pem` / `selfsigned-public-key.pem`): `localhost`,
  loopback, `tls_hostnames` and, once the port-mapping runtime knows it, the router's external
  address. It lists NO interface address. Every other peer, CGNAT included, and every
  listed trusted-proxy peer (loopback or not), gets it (`PeerScope::Public`).

`PeerScope::of(ip, trusted_proxy)` is the one classifier: a listed trusted proxy is Public first,
then canonical loopback is Host, then private/link-local/unique-local is Lan, else Public. The admin
route (`GET /api/admin/tls`) classifies the caller the same way from the connection's recorded peer
address (`TlsRuntime::admin_status_for`), so `fallback_fingerprint` is the fingerprint of the
certificate that admin's own browser reads (a host-machine admin sees the host-machine certificate).
A request with no connection record reports the LAN certificate.

The LAN and public certificates follow the same reuse, expiry, `tls_hostnames`, machine-hostname
opt-in and `--tls-regenerate` rules. The host-machine certificate lists nothing configurable, so
only expiry (within 30 days) or an unreadable file replaces it. `--tls-regenerate` replaces all
three and prints all three fingerprints. Interface churn never regenerates the public or
host-machine certificate, since they list none. The external address is polled each
maintenance tick (every 60 seconds, so a new mapping reaches the certificate at the next tick) from `PortMapRuntime::public_address` through `PortMapRuntime::external_source` (only a `Mapped` state with a publicly
routable address). A KNOWN external address the certificate lacks regenerates it once and the new
certificate lists only the current address; an address that becomes unknown leaves it unchanged, so a
router that briefly drops the mapping does not re-warn every player. A player who reaches the server
by a new address needs the new SAN anyway, so one extra warning at that moment is unavoidable. The
GM sees player addresses and shares their own; that is inherent and not covered. A port forward
(DNAT) is expected to preserve the player's source address, so the accepted peer would be the
player's public address; this is not yet checked on a real router. Remaining limit: a router that
rewrites the source address of forwarded connections to a private LAN address, and a
passthrough proxy on another LAN machine that is not listed in `trusted_proxies`, look like LAN
players and are shown the LAN certificate, which lists the host's interface addresses. A hairpin
connection from a LAN client is a LAN peer by definition.

Interface enumeration comes from `netdev`. If it fails, generation proceeds with `localhost`,
loopback and `tls_hostnames` and logs a warning; enumeration failure is never fatal for TLS. Every `tls_hostnames` entry is validated in `Config::validate_tls` (DNS name or IP
literal, no scheme/port/path, no wildcard except a leading `*.`); an invalid entry is a boot error.
A machine hostname or interface-derived SAN that rcgen rejects is SKIPPED with a warning.

Persistence: `selfsigned-cert.pem` and `selfsigned-key.pem` under `Config::tls_path()` (derived
through the database's sibling directory, never from `Config.db` directly). An in-memory database
with no `tls_dir` gets an in-memory certificate for the process (regenerated every boot; warned).
Key-file protection (`tls::keyfile`): Unix 0600 file; Windows a protected owner-only DACL through
the `windows` crate (SDDL `D:P(A;OICI;FA;;;<owner sid>)`). Only directories this code CREATES are
restricted (Unix 0700, Windows protected DACL): an operator-chosen existing `tls_dir`, or its
ancestors, is never re-permissioned (rewriting its ACL could lock out backup agents or the
operator). The key is written to `<key>.tmp`, protected while still empty, then renamed; a failure
to protect the key ABORTS the write (no key under an inherited ACL), removes the temporary file and
surfaces as `persist_error`, so the ladder serves an in-memory certificate and never blocks TLS. A
failure to restrict a directory this code created is a WARN naming the path (the key file's own
protection is what guards the secret). At start and at every regeneration or reload the served
source's and the fallback's SHA-256 fingerprints are logged at INFO, and an operator PEM's SAN
coverage and expiry are logged. The SHA-256 fingerprint is
`ring::digest` over the DER; SANs and expiry are read with `x509-parser`.

### 3.3 SNI-aware serving (`tls::resolver`)

`CertResolver` holds an immutable `Snapshot { primary, fallback, challenges, configs }` behind a
swap cell (`RwLock<Arc<Snapshot>>`); a hot swap installs a NEW snapshot and never mutates one.
`primary` is the operator (or ACME) leaf; `fallback` is the self-signed certificate (always
present). Selection is ONE pure function, `Snapshot::select(sni, alpn) -> Selected`:

- ALPN `acme-tls/1` selects `Selected::Challenge` (the challenge certificate for the SNI);
- else if `primary` is set, currently valid, and its DNS SANs cover the SNI (exact or single-label
  wildcard), `Selected::Primary`;
- else `Selected::Fallback`.

IP-literal and SNI-less connections therefore get the self-signed certificate that carries the IP
SANs; named connections get the trusted certificate. `primary`'s SANs and `not_after` are cached at
load so the check is a comparison.

Mechanism: the connection task awaits `LazyConfigAcceptor` to obtain a `StartHandshake`, then runs
two free functions with no shared mutable state: `hello_phase(&StartHandshake, &Snapshot) ->
Selected` (reads the hello, calls `Snapshot::select` exactly once) and `finish_phase(start,
Arc<Snapshot>, Selected, peer, trusted_proxy)` (calls `into_stream` with `snapshot.config_for(selected,
PeerScope::of(peer.ip(), trusted_proxy))`, the one place the peer scope is decided (`trusted_proxy` is `Config::is_trusted_proxy` for the peer), and builds
`ConnMeta::tls(peer, selected)`). The caller takes the `Arc<Snapshot>` from the swap cell ONCE and
passes it to both. `finish_phase` receives the snapshot by value and has NO handle to the swap
cell, so a second selection or a re-read is impossible by construction; a swap between the two
phases cannot make the presented certificate, `ConnMeta.cert` and the HSTS decision diverge. Each
`Selected` maps to its own pre-built `ServerConfig` in the snapshot (explicit ring provider): the
`Primary` and `Fallback` configs carry a fixed single-certificate resolver and ALPN `[h2,
http/1.1]`; the `Challenge` config carries the challenge certificate and ALPN `[acme-tls/1]`. The
hello wait and the finish share the connection's one deadline.

### 3.4 Operator PEM (`tls::pem`)

`tls_cert` + `tls_key` (both or neither). Validation: non-empty chain, the key loads under the ring
provider and matches the leaf, expiry and SAN coverage logged. A failure at boot degrades (3.5),
never aborts. Reload: file mtimes polled every 60 seconds (portable; no notify dependency, no
SIGHUP); a failed reload keeps the previous good certificate and logs; if the previous certificate
has since expired, the validity check routes to `fallback`.

### 3.5 Degradation ladder (`tls::TlsRuntime`)

BOUNDARY RULE: a boot error is permitted only when it is decidable from configuration VALUES alone —
syntax, pairing, contradiction — with no file or network I/O (`Config::validate_tls`, error type
`TlsConfigError`): PEM together with ACME; `tls = "off"` with PEM/ACME; one of `tls_cert`/`tls_key`;
`acme_domains` without `acme_terms_agreed`, with an IP/wildcard entry, or with an in-memory database
and no `tls_dir`; a malformed `tls_hostnames` entry; a malformed `public_url` (an absolute `https://`
URL with a host); an `acme_directory` that is not an `https://` URL; `acme_ca_root` without
`acme_domains`. Anything that needs file I/O or the network DEGRADES instead (invariant 2):

- Certificate: configured PEM (or ACME-issued) -> persisted self-signed -> in-memory self-signed ->
  TLS disabled for this process (the listener serves plain HTTP only and drops a `0x16` peer).
- An unreadable `acme_ca_root`, an unusable PEM, an unwritable `tls_dir`: the source degrades, the
  reason is recorded, plain HTTP is unaffected.
- A handshake failure affects that connection only.

`TlsStatus` (public in `GET /api/config`): `enabled`, `source` (`self-signed | operator | acme`),
`https_origins`, `alpn` (the ALPN list the server negotiates). The admin route `GET /api/admin/tls`
(`AdminUser`, a server-tier gate, never a world role) adds the fingerprint of the source
certificate and of the self-signed fallback (they differ once an operator PEM is loaded),
`not_after`, `degraded_from` and the last error, so a degraded PEM shows `source: self-signed`,
`degraded_from: operator`.

## 4. Per-request `Secure` and cookie names

### 4.1 Constraint

tower-sessions' `SessionConfig.secure` is one static value consumed only when the cookie is built;
its cookie controller receives no request. Its default cookie name is `id`.

### 4.2 Decider (`http::scheme`)

`classify(extensions, headers, config) -> Reach` (`Plain`, `Proxy`, `Native(Selected)`) is the one
decision; `request_scheme` and `hsts_max_age` are derived from it:

1. `ConnectInfo<ConnMeta>.tls` -> `Native(selected)`, scheme `Https`.
2. Else the immediate peer is in `trusted_proxies` (exact IP, the same test as `ClientIp`) and the
   RIGHTMOST comma-separated value of the LAST `X-Forwarded-Proto` header line equals `https` (ASCII
   case-insensitive) -> `Proxy`, scheme `Https`. Rightmost is the assertion of the nearest trusted
   proxy, so the operator must configure each proxy to set (overwrite) the header. A chain whose
   inner proxy overwrites to `http` yields `Http` (failing toward non-`Secure`, which still works on
   an https page). RFC 7239 `Forwarded` is ignored. An untrusted peer's header is ignored entirely.
3. Else `Plain`, scheme `Http` (also when `ConnectInfo` is absent, the axum-test mock).

### 4.3 Session layers (`auth::session`)

`SchemeSessionLayer` holds two session-manager layers over the same cloned store and the same signing
key: `http` (`with_secure(false)`, name `SESSION_COOKIE_PLAIN`) and `https` (`with_secure(true)`,
name `SESSION_COOKIE_TLS`), both `SameSite::Lax`, 7-day inactivity, `HttpOnly`. Each is wrapped in a
cookie manager; dispatch is on `request_scheme`. WebSocket upgrades traverse the same layers,
including HTTP/2 CONNECT.

### 4.4 Removal cookies

tower-sessions builds its logout removal cookie from the request cookie (name and value only), so the
removal carries no `Secure`; a browser ignores a `__Host-` Set-Cookie lacking `Secure`, and logout
would not delete the cookie. `normalise_host_cookies` runs on the HTTPS flavour's response and
rewrites every `Set-Cookie` whose name starts `__Host-` to guarantee `Secure; Path=/; HttpOnly;
SameSite=Lax` and no `Domain`. It applies to set and removal alike, so there is one mechanism. The
browser suite proves the effect (a logout over HTTPS removes the cookie from the jar).

### 4.5 Why distinct names

Cookies are not port-isolated, and a non-Secure cookie cannot overwrite an existing Secure cookie of
the same name, so a shared name breaks a browser that signs in over both schemes. The mixed-scheme
browser spec fails when the names are unified (mutation witness recorded in `HISTORY.md`). `__Host-`
additionally stops a plain-HTTP network attacker planting a cookie into the HTTPS session.
Consequences: one session per scheme; switching scheme means logging in again; origin-scoped
browser storage is separate per scheme.

## 5. HSTS

HSTS is per host: any TLS response carrying it converts every future `http://host:*` in that
browser to https for `max-age`, ending plain-HTTP play for that browser (against directive 1's
intent) and, on a self-signed certificate, turning the bypassable interstitial into a hard error.
Default: not sent. Opt-in `hsts_max_age_secs`. The gate, `hsts_max_age`, sends only when the request
is `Reach::Proxy` (the client sees the proxy's certificate, not ours) or `Reach::Native(Primary)`
(the CA-trusted primary was selected for that handshake); a connection that received the
self-signed `fallback` (IP/SNI-less access, an uncovered name, a degraded or expired primary) never
gets HSTS, even when `TlsStatus.source` says `operator`. No `includeSubDomains`, no `preload`.
Startup logs one WARN when `hsts_max_age_secs` is set, no trusted proxy is configured and the
active source is self-signed; it is not an error, because a proxy-fronted deployment legitimately
serves an internal self-signed certificate behind a trusted proxy.

## 6. ACME (not built)

Designed: `instant-acme` with `ChallengeType::TlsAlpn01`/`Http01`, a dedicated `reqwest`-backed
`HttpClient` (30 s timeout, no redirects, cookie-free; never the SSRF-guarded preview client, whose
blocklist would refuse private CAs), private-CA support through `acme_ca_root` added to the ACME
client's roots only, account credentials persisted under the `tls` directory, challenge certificates
published in the resolver's `challenges` map for `acme-tls/1`, HTTP-01 tokens served by an
unauthenticated `GET /.well-known/acme-challenge/{token}` for known tokens only, renewal check every
12 h with renewal under 30 days and a failed order backing off at least an hour, and a
hot swap of `primary`. Reachability is the operator's job: the CA validates on TCP port 443 for TLS-ALPN-01 (RFC 8737 section 3: "This connection MUST use TCP port 443") and TCP port 80 for HTTP-01 (RFC 8555 section 8.3), so the router must forward that external port to the Shadowcat port, or Shadowcat must bind it directly.

In the current build the `acme_*` keys are parsed and value-validated (`validate_tls`) and
`TlsRuntime::start_with` records `degraded_from: acme` with the reason
"ACME certificate issuance is not available in this build" whenever `acme_domains` is set; an
unreadable `acme_ca_root` degrades the same way with its own reason.

## 7. Absolute URLs and origins

- The server builds no invite or absolute URLs. The audio-monitor default allow-list admits the four
  origins `http(s)://localhost:30000` and `http(s)://127.0.0.1:30000`, derived from one constant list.
- `tls.https_origins` publishes only explicitly configured origins: `public_url` if set; else one
  entry per ACME domain, per `tls_hostnames` DNS entry (`https://<name>:<bind port>`), and per
  non-wildcard DNS SAN of the loaded operator PEM leaf. The machine hostname is never published (it
  can be unreachable, and disclosing it is a needless leak). With none published, the client links to
  its OWN location with the scheme swapped, which is by construction the address that just worked;
  with some published, it prefers a published origin whose host the player is already using.

## 8. Listener (`tls::listener`)

`SniffListener` implements `http::conn_serve::ConnSource`: `Io = ConnStream` (`Plain(TcpStream)` or
`Tls(Box<TlsStream<TcpStream>>)`), yielding `(ConnStream, ConnMeta { peer, tls, cert })` (`cert` is
the single `Selected` recorded for a TLS connection, `None` for plain; `peer` is canonicalised, an
IPv4-mapped IPv6 address reduced to IPv4). `accept` returns `Ok(None)` only if the acceptor task
died, which ends the serve loop with an error log; it never returns an `Err`, because the acceptor
task paces its own accept errors.

- A background acceptor task owns the `TcpListener`; its loop selects between `accept()` and the
  receiver closing, so shutdown releases the port immediately, and `Drop` aborts the task. Accept
  errors follow axum's own policy (`http::conn_serve::accept_error_pause`): connection-level errors
  ignored, others logged then a 1 s sleep; the loop never breaks on error. `serve_router` consults
  the same function for a `ConnSource` that returns an `Err` and holds the pause itself, because it
  drops the `accept` future on every reaped connection task.
- Each accepted socket runs, under ONE 10 s deadline, `peek` and — when the byte is `0x16` — the
  two-phase handshake of 3.3. `TcpStream::peek` consumes nothing, so the plain path needs no prefix
  buffering. A client that connects and sends nothing is dropped after 10 s. The snapshot is taken
  after the ClientHello arrives (`sniff_and_hello`, then `finish`), so a swap that lands while the hello is in flight
  is honoured, and both phases use it.
- First-byte deadline (decision): there is no separate, shorter first-byte deadline; the single
  10 s deadline covers sniff and handshake. Evidence: (1) browsers open speculative connections
  that send no bytes until a request needs them and close any that goes unused for about 10 s
  (web.dev, "Establish network connections early to improve perceived page speed", caution on
  `rel=preconnect`), so a silent socket up to that age is normal browser behaviour and 10 s is the
  longest silence worth holding; a shorter deadline would drop sockets a browser is about to use
  (costing it one reconnect round trip) and a longer one only lets a silent peer hold a cap slot
  longer. (2) Measured here: with a raw TCP listener behind a page carrying a preconnect link,
  Playwright's Chromium and Firefox opened no zero-byte connection within 25 s, so the page-level
  preconnect path produced no silent socket against this port. Not verified: real omnibox
  preconnect (it cannot be automated). The per-peer cap bounds how many silent sockets one peer
  can hold meanwhile.
- Admission (`Limits`): a global cap of 1024 in-flight pre-hand-off connections and a per-peer cap
  of 32 for ordinary peers, keyed by `peer_key` (canonical IPv4, or the /64 prefix of an IPv6
  address, RFC 4291: one IPv6 subscriber holds a whole /64, so a per-address key would give one host
  2^64 budgets); a peer for which `Config::is_cap_exempt` holds (a peer listed in `trusted_proxies`,
  per `Config::is_trusted_proxy` canonicalised on both sides, or a loopback peer) is exempt from the
  per-peer cap (every player behind that peer shares its address) and bounded by the global cap
  only; over-cap sockets are closed immediately. `is_trusted_proxy` stays the forwarded-header trust
  decision and has no loopback arm: a loopback peer is cap-exempt but its `X-Forwarded-*` headers
  are still ignored unless it is listed. A permit lives from accept until the finished
  `ConnStream` is handed to `accept()`. The cap uses the peer address because it runs before any
  header exists, which is why proxy and loopback peers are exempt.
- Post-hand-off bounds (`http::conn_serve`): the caps bound the unauthenticated, cheap-to-create
  phase; afterwards time bounds apply. `axum::serve` installs no timer, so hyper's header-read
  timeout and HTTP/2 keep-alive would be inert; the serve loop is therefore owned by
  `conn_serve::serve_router`, which builds each connection with a `TokioTimer`, a 30 s HTTP/1
  header-read timeout (armed on every wait for a request head, so it also closes an idle keep-alive
  connection; it does not apply after a WebSocket upgrade, to bodies or to a running handler) and
  HTTP/2 keep-alive pings (30 s interval, 20 s ack timeout). The HTTP version is decided by the
  negotiated ALPN and never by reading the connection, and hyper's `http1`/`http2` connection
  builders are used directly: hyper-util's auto builder reads a 24-byte preface under no timer
  (and ignores its `http1_only`/`http2_only` when upgrades are enabled), so a peer sending `P` and
  stalling would hold the connection forever. A plain connection therefore speaks HTTP/1.1 only
  (HTTP/2 with prior knowledge, "h2c", is not supported; no browser uses it), and a TLS connection
  speaks what ALPN negotiated, HTTP/1.1 when it negotiated nothing. hyper arms no timer during the
  HTTP/2 server handshake either, so an `h2` connection's 24-byte preface is read under the
  header-read bound by `conn_serve` itself (racing the drain signal, so a silent peer never delays
  a shutdown) and replayed to hyper.
  - HTTP/2 idle bound: hyper has no HTTP/2 idle timeout, and its server keep-alive (`keep_alive_while_idle`
    is true on the server, read in hyper's `proto/h2/server.rs`) only closes a peer that stops
    acknowledging pings. `conn_serve` therefore counts the requests in flight on each connection
    (`LeaseService`: a lease per request, held until the response body is dropped) and calls
    `graceful_shutdown` once a connection has had none for `ConnLimits::h2_idle` (60 s, above the 50 s
    a missed ping takes to surface). A WebSocket over HTTP/2 is an extended-CONNECT stream that hyper
    runs as a detached upgrade task, invisible to a request counter, so `ws_handler` takes the
    request's `ConnLease` from its extensions and moves it into the session task: a live session keeps
    its connection busy.
  - Live-connection caps: `ConnLimits::max_live` (2048) and `max_live_per_peer` (256) bound
    connections from hand-off until the connection task AND every `ConnLease` have ended, so an HTTP/1.1
    WebSocket session (detached from its connection) keeps its slot. Keying and exemption are the
    accept-time caps': both use `http::peer_slots::PeerSlots` and `peer_key`, and the cap
    exemption (`Config::is_cap_exempt`) is decided once by `SniffListener` and carried as `ConnMeta::cap_exempt`. A connection
    over a cap is closed unread. The limits are chosen, not measured: a table needs tens of
    connections (one HTTP/2 connection, or up to about six HTTP/1.1 ones plus a WebSocket, per
    player), so 256 admits a large shared NAT while stopping one host from holding the server;
    neither is a configuration key.
  - Drain: a serve loop whose connection source ends (as opposed to a shutdown request) drains like
    a shutdown and `http::serve` returns an error, so the process exits non-zero and a supervisor
    restarts it. Both causes first run `serve_router`'s `on_drain` hook (`WsState::trigger_shutdown`,
    the single place sessions are told to close), because an HTTP/2 connection resolves only when its
    streams end and a WebSocket is one. The drain has ONE deadline, `ConnLimits::drain_timeout` (5 s,
    `SHUTDOWN_DRAIN_TIMEOUT`) after the hook ran, returned as `ServeOutcome::drain_deadline`; both
    waits of `serve_with` use it. At the deadline `serve_router` fires a kill signal that every
    connection's IO (`Replay`, in all five of `poll_read`, `poll_write`, `poll_write_vectored`,
    `poll_flush`, `poll_shutdown`) answers with an I/O error, so hyper ends the connection through
    its own error path. A connection whose IO was failed and that has not ended `KILL_GRACE` (2 s)
    later is dropped by `drive` and counted in `DrainReport::dropped`, and a task still running
    `ABORT_WAIT` (`KILL_GRACE` plus 1 s) after the failure (one not in `drive`) is aborted and
    counted in `DrainReport::aborted`, each at error level (a connection that does not end when its
    IO errors is a defect); a panicking connection task is reported in `DrainReport::panicked`. Sessions hyper
    cannot see are then waited for `max(remaining, SESSION_GRACE)`, `SESSION_GRACE` being the Close
    bound plus one second, so `serve_with` returns within `drain_timeout + SESSION_GRACE`
    (`+ ABORT_WAIT` only when a task outlives its killed IO).
  - Stalled peers. One window, `Config::egress_window` (`egress_window_secs`, 60 s by default, 1 to
    3600), bounds every unit below; it reaches `ConnLimits::egress_window` and `WsState::bounded_sink`
    from one method, so the response bounds and the WebSocket bound cannot be tuned apart. The
    contract: **a response is cut when its peer returns no flow-control credit for a pending chunk,
    or the socket accepts nothing, for the window, or a finished response's own stream makes less
    than `min(egress_floor, bytes still queued)` of progress over the window.** A cut lands between
    the window and the window plus one sampler tick (`egress_tick`: a window divided by
    `EGRESS_TICKS_PER_WINDOW`, 4, so a quarter window) after the last progress: within 75 s at the
    default. Connections with nothing in flight are never cut by these units, and
    `h2_idle` and keep-alive are unchanged.
    - Stream unit: a response body is sliced into frames of at most `EGRESS_CHUNK_BYTES` (16 KiB,
      one HTTP/2 frame), so a stalled peer is visible at a frame boundary. A stream whose pending
      frame has waited for credit for the window is reset (CANCEL); its connection, its other
      streams and every tunnel are untouched. The wait is timed only while hyper is not polling the
      producer, so a sparse producer with a healthy reader is never cut.
    - Ended-unflushed unit: a response whose body has ended hands its stream's handles back, so it
      cannot be reset. hyper hands a chunk to h2 only while the stream's send buffer holds less than
      the credit the peer granted, so what stays queued behind a stalled window is at most one chunk
      (a larger body parks its next chunk with the body still open, and the stream unit acts). A
      table of the connection's live streams, fed from the frames on the wire in both directions
      (`egress::table`), tracks each finished response's own queued tail; a tail whose own stream
      writes less than `min(egress_floor, bytes still queued)` over the window closes the
      connection. Only a stream's own DATA counts as its progress: not another stream's, a tunnel's,
      a PING's or any other write. The table is created at parse time, shrunk only on wire evidence
      that the stream ended, purged at a GOAWAY, and fails closed: a connection whose table outgrows
      its cap (`H2_MAX_CONCURRENT_STREAMS` plus a derived parse lag) is cut and counted as
      `tracker_overflow`.
    - Connection unit: HTTP/1 and an HTTP/2 writer parked on a socket that accepts nothing. The
      sampler (an arm of `drive`'s `select!`, so a parked writer is re-observed with no other
      wake source) closes a connection whose writer stayed parked for the whole window with fewer
      than `egress_floor` bytes accepted (HTTP/2: DATA payload bytes only, so PING and SETTINGS
      acknowledgements are not progress). The socket accepts data in lumps, not steadily: on Windows
      loopback the lumps measured were about 100 to 520 KB, and on a bare socket pair their size
      followed the reader's receive buffer. So a reader that consumes less than about one lump per
      window (roughly 2 to 9 KB/s at the 60 s default) can be cut while it is still reading. Real
      network paths are unmeasured; their steps are expected to be finer. Operators on extremely
      slow networks raise `egress_window_secs`.
    - Measured stream-unit boundary (the `h2` crate as client, loopback, a paced reader returning
      credit as it reads, `/endless` body): at a 1 s window and stream windows of 1 MiB and 6 MiB
      (six runs per rate) a reader at 8 to 20 KiB/s was cut in every run and one at 40 KiB/s or
      above in none. At 24 and 28 KiB/s it was cut in some runs and not in others at both windows;
      at 32 KiB/s it was cut in one of six runs at 1 MiB and in none at 6 MiB. At a 32 KiB stream
      window (three runs per rate up to 32 KiB/s, two runs from 40 KiB/s) every rate up to 24 KiB/s
      was cut and 32 KiB/s and above was not. At a 3 s window, at stream windows of 64 KiB and 1 MiB
      (six runs per rate), rates of 2.7 and 4 KiB/s were cut and 8 and 10.7 KiB/s were not. One
      16 KiB chunk per window works out to 5.3 KiB/s at 3 s, a figure derived from the chunk size
      and not run; 10.7 KiB/s was run, and reading it as two chunks per window is derived. The
      default 60 s window was not measured, and neither was a browser's window or credit batching;
      raising `egress_window_secs` lowers the rate that is cut.
    - Consequences: a reader that returns credit for less than about one chunk per window is cut; a
      cut `export_world` download restarts from zero (it has no length and no `Range`).
    - Memory per stalled stream: `H2_SEND_BUF_BYTES` (160 KiB, hyper's default is 400 KiB) bounds the
      body bytes polled ahead of an unread peer to the buffer plus one chunk (176 KiB), and the
      unsent backlog behind a socket that accepts nothing. The throughput cost of that setting is
      judged by the median over runs of the download-time ratio against the default (at least
      0.90 on every operating system; the recorded medians are in the constant's documentation)
      and recorded per operating system by `tests/h2_send_buf_gate.rs`, which CI runs in a release
      build and which also asserts a per-run floor below the worst recorded run.
    - No per-content-type exemption: a receiver that stops reading by choice (a paused media
      element) is cut like any other.
    - WebSocket sends: every send goes through `ws::bounded_sink::BoundedSink`, built by
      `WsState::bounded_sink`, the one constructor (pre-join refusals, the Welcome and every later
      frame). A send pending for the window fails as stalled and ends the session, releasing its
      `ConnectionGuard` and `ConnLease` (the connection's cap slot); once shutdown is signalled a
      pending send gets only `CLOSE_SEND_TIMEOUT` (2 s). The bound is on one pending flush, not on
      progress through a frame: a frame of F bytes needs at least 8F / window bits per second
      (the largest, `WS_MAX_MESSAGE_BYTES`, about 1.7 Mbps at 60 s), because the socket
      (`axum::extract::ws::WebSocket`) exposes no byte counter. `WsState::pending_sends` counts the
      sends pending now, and the shutdown drain warning reports it as `blocked_sends`.
    - A WebSocket inside an HTTP/2 stream: hyper's tunnel task hands every write to h2 whether or
      not the peer granted credit, and h2 buffers beyond the window without bound, so such a send
      never went pending: with a peer returning no credit the heap grew by the volume broadcast
      (measured with a counting allocator: about 24 MB per 400 frames of 60 KiB) and the stall bound
      never started. `ws::tunnel_gate::TunnelGate`, under the sink, compares the bytes the session
      wrote with the DATA payload bytes the connection's writer accepted for the session's own
      stream (`TunnelProbe`, in every HTTP/2 request's extensions) and stops accepting frames while
      the session is more than `H2_SEND_BUF_BYTES` ahead; the pending send is then bounded like any
      other, and a reader that takes a frame in less than the window keeps its session. With the
      gate the heap is flat under the same load.
    - Residuals: a producer that pauses while its stream is stalled and holds nothing queued stays
      open until it next produces; a tunnel's bound is per pending flush, as above.
  - Vendored `h2` (`vendor/h2`, 0.4.19 plus `SHADOWCAT-PATCH.diff`, verified by `pnpm lint:vendor` and
    `lint:vendor:upstream`): four changes in six files. (1) Two arms in `State::recv_eof` and
    `State::handle_error` turn a `ScheduledLibraryReset` stream into an error-closed one when the
    connection ends, so its slot is released (a stalled tunnel cut at the deadline otherwise trips the
    debug `Counts` drop assertion). (2) Every server request carries the public `h2::StreamId` as an
    extension, which the egress stream table correlates with wire frames. (3) `Connection::poll2`
    stops reading frames while `H2_MAX_PENDING_ACCEPT_RESETS` reset streams await accept and resumes
    as hyper accepts, so a cancel burst of any size is served and pending resets pin at most that many
    header blocks. (4) A sustained reset-rate rule ends a connection that resets streams faster than
    16 times that bound per 10 s with GOAWAY(ENHANCE_YOUR_CALM); it is the rapid-reset protection and
    its numbers derive from the one reset limit hyper forwards.
  - In-flight requests at the deadline are cut off, not finished: their futures are dropped at the
    next await. The mutating handlers are ordered to survive that (asset replace commits the row
    before swapping the file; uploads stage into `*.tmp`; a backup writes `manifest.json` last and
    restore refuses a directory without it; export only reads). The residue is stray `*.tmp` files.
  - Refusals: `PeerSlots::admit` counts a refusal and logs one summary warning per
    `REFUSAL_WARN_INTERVAL` (10 s) with the count and one refused key (an IPv4 address or an IPv6
    /64); the state is a counter, never a map keyed by the attacker-chosen peer. A per-peer refusal
    also tells the operator to list a reverse proxy in front of the server in `trusted_proxies`.
  - Descriptors: `fd_budget` raises the soft `RLIMIT_NOFILE` at startup to exactly
    `descriptors_needed` (the two caps plus `descriptors_reserved`; never above the hard limit, on
    macOS bounded by `kern.maxfilesperproc`, queried at runtime; a limit that already covers the
    need is left alone), reads the soft limit back and fits the pre-hand-off cap and `max_live`
    together inside `soft - descriptors_reserved()`, keeping their ratio with floors, and warns
    once when it reduced them. `descriptors_reserved` is `FD_RESERVE` (512), the hand-off queue's
    `HANDOFF_QUEUE_CAP` (128: a connection waiting in it holds a descriptor and belongs to neither
    cap) and `STREAMED_FILE_PERMITS` (256: the size of the semaphore, `AppState.file_permits`, that
    every streamed asset file takes a permit from). `http::assets::permits::StreamedFilePermits`
    owns the pool: `serve_file` (all four stored-file sites) and `export_world` call
    `acquire`, a bounded wait of `PERMIT_WAIT` (2 s) that ends in `AppError::Unavailable` (503 with
    `Retry-After: 1`) before any header is written. A GET takes the permit before it opens the file;
    the permit moves into the body and is returned when the last file byte has been read (not when
    the last buffered byte has been sent) or when the body is dropped, which a stall cut, a reset or
    a drain does. A HEAD of a version-validated arm, a 304 answered before the file is touched and a
    416 hold none. A HEAD of the byte-validated arm opens the file for the head read, so it takes a
    permit for that read and returns it with the descriptor: no asset file is open outside the pool.
    A scene of 150 distinct images was measured at 3 to 4 permits at its peak over HTTP/1.1 and
    HTTP/2, which is why 256 stands. The raise and the fit read the
    same definition, so they cannot disagree, and the pool is created from the same constant. A platform without the limit is not
    clamped.
- `ConnectInfo<ConnMeta>` is inserted in exactly two places: `conn_serve::serve_router` and
  `http::into_service` (for `axum::serve` transports, through `Connected` for
  `IncomingStream<TcpListener>`), so every serving path yields the same `ConnMeta`.
- `tls = "off"`: same listener, no acceptor; a `0x16` first byte is dropped.

### 8.1 HTTP/2 and WebSocket

`axum` is built with its `http2` feature (without it `axum::serve` speaks HTTP/1 only, and advertising
`h2` would break every h2-negotiating browser). axum enables the extended-CONNECT protocol and
`WebSocketUpgrade` accepts HTTP/2 only for `CONNECT`, so `/ws` is routed
`get(ws_handler).connect(ws_handler)`. The compression layer covers every route and excludes status `101`
and any response carrying `Accept-Ranges` (8.2); axum's HTTP/2 WebSocket reply is `200` with an
empty body, which the layer's default predicate does not compress. Evidence: an integration test opens an h2 connection, sends extended
CONNECT with `Accept-Encoding: gzip, br` and completes an intent round trip (fails if the CONNECT
route is removed or the compression layer mangles the reply); a predicate test pins the empty-body
case; a browser spec on each engine reads `TlsStatus.alpn` from `GET /api/config`, asserts the page's
resources negotiated a protocol contained in it (`nextHopProtocol`), asserts `h2` when it is offered,
and asserts the WebSocket welcome arrives. No copy of the ALPN list exists in TypeScript.

### 8.2 Asset responses: validators, ranges and compression

Every stored-file response (`http::assets::serve`'s canonical, `thumb`/`preview`, `opus`,
`opus-webm` and `sheet` arms, and `http::assets::mutate::original`) goes through one function,
`file_response::serve_file`, so the validator order, the range decision and the framing cannot
differ between sites. The file is read in 64 KiB pieces as the connection pulls them; no response
holds a file in memory, and `LeasedBody` re-chunks to the egress frame size.

- Order, canonical, `thumb`, `preview`, `sheet` and the original (validators derived from the
  record): authorisation (`permission_context`; `gm_asset` for the original), then a matching
  `If-None-Match` is answered 304 with `ETag` and `Cache-Control` and no range headers before any
  file access and holding no permit, then (canonical, `thumb`, `preview`) `ensure_derivative`, then
  the streamed-file permit (8, Descriptors), then the file is opened and its length read from that
  same handle, then `decide`. Order, `opus` and `opus-webm` (the validator is derived from the
  served file, `Validator::OfFile`): authorisation, then the permit (a bounded wait, 503 under
  starvation), then the file is opened and its length, modification time and a SHA-256 of its first
  4 KiB are read from that handle into the `ETag`, then a matching `If-None-Match` is answered 304,
  then `decide`. A 304 on this arm therefore takes a permit and is refused with 503 when the pool
  is exhausted; a `Range` request never bypasses that order. A GET streams from the handle it
  opened: an asset replaced or deleted mid-response is not waited on and the open response keeps
  the old bytes. A HEAD of a version-validated arm calls `fs::metadata`, takes no permit and
  answers the GET's status and headers with an empty body; a HEAD of a byte-validated arm opens the
  file for the head read alone, under a permit held for that read. Validators: `"{id}-{version}"` (canonical,
  `thumb`, `preview`, `sheet`), `"{id}-{version}-orig"` (original), and for the derivatives
  `"{id}-{version}-opus-{len}-{mtime}-{head hash}"` and `"{id}-{version}-opus-webm-{len}-{mtime}-{head
  hash}"` (length and modification time in hexadecimal, eight bytes of the head hash in hex): an
  in-place rewrite of a derivative (the WebM upgrade sweep swaps files without a version bump)
  changes the validator, so a cached copy and an `If-Range` resume across the swap both restart.
- One classification, `range::classify(content_type, disposition, route)`: `Route::Original` is
  `DownloadFull` for every type. On `Route::Embed` an `audio/` or `video/` type is `MediaSliced`
  whatever its disposition (a media element fetches the canonical file with `Range` and reads ahead
  of playback, so its response must be a slice that never idles past the egress window); any other
  type in the ranged list is `MediaSliced` when served inline and `DownloadFull` as an attachment;
  any other type is `Ignore`. The list is
  `range::is_media_or_archive`: prefixes `audio/`, `video/`; `image/` except `image/svg+xml` (the
  exclusion tower-http's own image predicate encodes; a parity test pins them); exact
  `application/ogg`, `application/pdf`, the archive types (zip, gzip, bzip2, xz, zstd, 7z, rar) and
  `font/woff`, `font/woff2`. `serve`'s attachment disposition for non-raster types is a
  rendering-safety header and does not make a compressible type a download; only the original
  route does.
- Decision (`range::decide`, RFC 9110 semantics, pure): `Ignore`, no `Range`, a unit other than
  `bytes` or any syntax error answer `Full`. `If-Range` honours `Range` only on an exact strong
  `ETag` match. More than 16 range-specs answer `Full`; unsatisfiable specs are dropped, none left
  is 416 with `Content-Range: bytes */len`; satisfiable ranges are sorted and coalesced (overlapping
  and adjacent) so the total never exceeds the file. One part is a 206; several are
  `multipart/byteranges` with an exact `Content-Length` and a `shadowcat-` plus v4 UUID boundary.
  `MediaSliced` shortens a single part to 2 MiB (`MAX_RANGE_SLICE`) and answers `Full` when a
  multi-part total exceeds it, never truncating a part among several. `DownloadFull` has no byte
  cap.
- Compression follows the same decision through one header: `serve_file` writes
  `Accept-Ranges: bytes` exactly when the class is not `Ignore`, and
  `compression::AssetAndWsSafePredicate` refuses any response that carries it, so no second list of
  types exists. `tests/assets_compression.rs` pins every (type, disposition, route) row through the
  real router with `Accept-Encoding: gzip`. A ranged response cannot be compressed (a range
  addresses bytes of the selected representation, and gzip output is not reproducible, so
  `If-Range` and a strong `ETag` would stop meaning anything); compressing only unranged types keeps
  SVG and text compressed, and the cost is that `audio/wav` and `audio/aiff` assets and the
  attachment route lose compression.
- The body stream is fused: tower-http's compression adapter reads its source through a reader that
  is not, and a stream polled after its end must answer the end.

## 9. Configuration surface

Four-layer precedence (CLI `--kebab-case`, env `SHADOWCAT_UPPER_SNAKE`, TOML key): `tls` (`auto` |
`off`, default `auto`), `tls_cert`, `tls_key`, `tls_hostnames` (list), `tls_dir` (default `tls/` beside
the database), `public_url`, `acme_domains` (list), `acme_ca_root`, `acme_email`, `acme_terms_agreed`
(default false), `acme_challenge` (`tls-alpn-01` | `http-01`), `acme_directory`,
`hsts_max_age_secs`, `tls_advertise_hostname` (default false; see 3.2). One-shot CLI:
`--tls-regenerate`. `trusted_proxies` is `Vec<IpAddr>`, parsed at figment extraction (an entry that
is not an IP address fails boot naming the key and the entry); `public_url` must be exactly an
`https://` origin (`config::https_origin_of`: no user information, no path but `/`, no query, no
fragment) and is published as that origin's ASCII serialization. List-valued env keys keep the bracket syntax
caveat. `bind` default stays `127.0.0.1:30000`.

Retired-key guard: after the figment merge and before extraction, `Config::load` fails if the
merged figment contains `behind_tls_proxy` (the env provider lowercases
`SHADOWCAT_BEHIND_TLS_PROXY`, so env and TOML are both covered; clap rejects the old CLI flag). The
message names the rename, says to delete the key even when its value was `false`, and gives the full
migration: set `behind_proxy`, list the proxy in `trusted_proxies`, and make the proxy OVERWRITE
`X-Forwarded-Proto`. `--backup-to` and `--restore-from` also pass through `Config::load`, so a stale
variable blocks them too, intentionally; `shadowcat audio-monitor` runs before `Config::load` and is
unaffected. `Config::validate_tls` is NOT part of `Config::load`: it runs only on the entry points that
use TLS settings (serving, `--tls-regenerate`, `test_server`), so a database backup or restore is
never blocked by an unrelated TLS mistake in the same configuration file. A separate WARN
when `behind_proxy` is true and `trusted_proxies` is empty.

## 10. Client

1. `ServerConfig` (`GET /api/config`) carries `tls: TlsStatus`: ONE field list shared by the Rust
   type, the generated ts-rs type and the Zod mirror (`ServerConfigSchema`, whose drift test fails
   when any copy lacks a field). No fingerprint.
2. Notice: on an `http:` non-loopback origin with `tls.enabled` the entry view shows a dismissible
   notice with the HTTPS link(s); the dismissal persists per origin in guarded `localStorage`
   (storage can be unavailable). It never blocks login and never redirects. For a self-signed source
   it adds "your browser will warn once". The decisions live in `@shadowcat/core`'s `https-access`
   module so the entry view and the modules cannot disagree.
3. WebSocket hint: `WsClient` reports consecutive connection failures before its first welcome
   (`onConnectFailure`); at the third failure on an `https:` page whose certificate source is
   self-signed the shell notifies with the `http://` address of the same server.
4. Microphone message: `ducking.micSource.denied.unavailable` for a server without TLS;
   `ducking.micSource.denied.unavailableHttpsAvailable` (with the `https://` address) when
   `tls.enabled`.
5. The OS-monitor section shows a hint on `https:` pages while the monitor is not connected (a
   mixed-content `ws://` connection is refused by WebKit).
6. The `randomUuid` fallback stays (plain HTTP is supported).

## 11. Browser HTTPS-upgrade features

Some browsers can rewrite a typed `http://` address to `https://` against the shared port; the
server answers TLS there, so an upgrade reaches the self-signed certificate — a bypassable
interstitial, unlike HSTS. Acceptance rule: a dead end on `http://<lan-address>` in the DEFAULT
configuration of a mainstream browser violates directive 1 and cannot be closed with a documented
workaround; it sends this design back for review. A workaround may be documented only for
non-default configurations.

Measured (Playwright engines, the machine's real non-loopback address; the outcome table is in the
hosting guide): Chromium with default settings does not upgrade an explicit `http://` navigation
(no TLS attempt reaches the server; the `HttpsUpgrades` launch feature applies to scheme-less
omnibox input only); Chromium with HTTPS-First Mode (profile preference, new headless mode)
upgrades both a bare IP and a DNS name, the attempt fails on the certificate, and the navigation
is blocked with `ERR_BLOCKED_BY_CLIENT`; Firefox HTTPS-Only Mode exempts private
addresses by default (loads over `http://`) and upgrades a DNS name or, with `upgrade_local`, a
private address, landing on the certificate error page, which is bypassable; WebKit loads over
`http://`. The HTTPS-First prompt ("This site doesn't support a secure connection", with "Go back"
and "Continue to site") is browser UI, not web content: in Playwright's Chromium (new headless), a
windowed Chrome for Testing and over `connectOverCDP` the tab's document is an empty
`chrome-error://chromewebdata/` page (one accessibility node, blank screenshot), and DOM,
accessibility-tree, DevTools mouse and DevTools keyboard input never reach it, while a capture of the
browser window shows it. On Windows its "Continue to site" button is invoked through UI Automation
(`invokeBrowserButton`), which reaches the plain-HTTP app; that click is automated. On macOS the
same function drives System Events through JavaScript for Automation, and on Linux AT-SPI through
PyGObject under a virtual display; the `ui-e2e-https` CI job runs the interstitial specs with the
matching driver on each of its three OS legs. The certificate error interstitial of a direct
`https://` navigation does have a DOM and its "Advanced" / "Proceed" click is automated. Not verified: real Safari, real
Android Chrome and any browser's behaviour for a public IP address.
Outcomes are asserted per (project, platform); the address under test must be private (RFC 1918),
because engines upgrade CGNAT and public addresses like internet hosts: on such an address the
tests skip with a message locally and FAIL on CI. A (project, platform) cell with no measurement
is an `UNMEASURED` annotation locally and FAILS on CI with the observed value in the message, so
the first CI run on each OS produces the measurement to record in the table (`judgeOutcome`).

## 12. Modules and files

`tls` (`mod`: `TlsRuntime`, `TlsStatus`, `ConnMeta`, `Selected`, `ALPN_PROTOCOLS`, the ladder;
`listener`: `SniffListener`, `ConnStream`, `Limits`; `resolver`: `CertResolver`, `Snapshot`,
`hello_phase`, `finish_phase`; `selfsigned`; `pem`; `keyfile`), `http::scheme`, `auth::session`
(`SchemeSessionLayer`, `normalise_host_cookies`), `config` (keys, retired-key guard,
`validate_tls`, `tls_path`), `main` (ladder, provider install, listener binding, URL logging,
`--tls-regenerate`), `bin/test_server` (through `into_service`), and the `shadowcat-test-support`
crate's TLS harness. Tests live in sibling files.

## 13. Crates and licences

| Crate | Licence | Notes |
|---|---|---|
| rustls 0.23 | Apache-2.0 OR ISC OR MIT | `default-features = false`, `ring`, `std`, `tls12` |
| tokio-rustls 0.26 | MIT OR Apache-2.0 | `default-features = false`, `ring`, `tls12` |
| rcgen 0.14 | MIT OR Apache-2.0 | `default-features = false`, `pem`, `ring` |
| x509-parser 0.18 | MIT OR Apache-2.0 | SAN and expiry reads |
| netdev 0.46 | MIT | interface enumeration |
| ring 0.17 | Apache-2.0 AND ISC | SHA-256 fingerprint |
| tower-cookies 0.11 | MIT | `Set-Cookie` access for the normaliser |
| gethostname 1.1, time 0.3 | Apache-2.0, MIT OR Apache-2.0 | machine name; validity timestamps |

All transitive crates added by these are MIT, Apache-2.0, Zlib or combinations of them; the one
`CDLA-Permissive-2.0` crate (`webpki-roots`) is reachable only through a test-support dev dependency
and is absent from the release binary. `aws-lc-rs` is absent (`pnpm lint:crypto-backend`).

## 14. Rejected alternatives

Secure from static bind (wrong in mixed deployments); redirect (directive 1); HSTS by default; one
shared cookie name; two ports; `axum-server` (a second serving loop re-proving shutdown/drain);
`rustls-acme` (a second async stack owning the accept path); instant-acme's default client (platform
verifier); a local CA (a per-device root install is heavier than one click-through and hands the host
MITM power over the device); DNS-01 (per-provider integrations); TLS only when a certificate is
supplied (directive 2); accepting the old `behind_tls_proxy` key as an alias (a compatibility shim on
a security flag); regenerating the certificate on every interface change (interstitial churn).
