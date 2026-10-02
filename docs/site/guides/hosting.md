# Hosting a Shadowcat server

Shadowcat ships as **one native executable**: the Rust server with the entire web
client embedded inside it. Hosting a game means running that one binary and
pointing browsers at it — no database server, no web server, no separate client
deploy.

## Get the binary

**From a release:** download the artifact for your OS — a macOS `.app` bundle, a
Linux staging tree with a `.desktop` entry, or a Windows `shadowcat.exe` (icon
embedded).

**From source** (needs Rust stable with the `wasm32-unknown-unknown` target —
`rustup target add wasm32-unknown-unknown` — Node 22, pnpm 9):

```bash
pnpm install
pnpm build
cargo build --release --manifest-path src/server/Cargo.toml
```

The order matters: the server **embeds** the client bundle at compile time, so
`pnpm build` (which produces `dist/`) must run before any `cargo build` of the
server. The binary lands in `target/release/` (`shadowcat` /  `shadowcat.exe`).

## First run

```bash
./shadowcat
```

The log prints `shadowcat listening` with the bind address — default
`127.0.0.1:30000`. Open `http://localhost:30000/` in a browser (`https://localhost:30000/` also
works and shows a one-time certificate warning).

With no admin account yet, the app shows a **setup form** that creates the first
admin. Whether that form needs a token depends on the `setup_token` policy:

| `setup_token` value | Behavior |
|---|---|
| `auto` *(default)* | Loopback bind → open setup (you're on the same machine). Any other bind → a one-time token is **generated and printed in the server log**; the form requires it. |
| `off` | Setup form is open until an admin exists (the log warns if you do this on a non-loopback bind). |
| `required` | Token required; generated and printed at boot. |
| any other string | That string **is** the token. |

**Headless bootstrap** (remote hosts, containers): set `admin_user` +
`admin_password` in config and the admin account is seeded at boot — no setup
form involved.

## Configuration

Four layers, highest wins:

1. **CLI flag** — `--bind 0.0.0.0:30000`
2. **Environment** — `SHADOWCAT_BIND=0.0.0.0:30000` (any key, upper-cased,
   `SHADOWCAT_` prefix)
3. **TOML file** — `bind = "0.0.0.0:30000"` in `shadowcat.toml` (working
   directory, or `--config <path>`; a missing file is ignored)
4. **Built-in default**

The same setting, three equivalent ways:

::: code-group

```bash [CLI]
./shadowcat --bind 0.0.0.0:30000 --db /srv/shadowcat/shadowcat.db
```

```bash [Environment]
SHADOWCAT_BIND=0.0.0.0:30000 SHADOWCAT_DB=/srv/shadowcat/shadowcat.db ./shadowcat
```

```toml [shadowcat.toml]
bind = "0.0.0.0:30000"
db = "/srv/shadowcat/shadowcat.db"
```

:::

Full key reference (flag = `--<key>` with dashes, env = `SHADOWCAT_<KEY>`):

| Key | Default | Meaning |
|---|---|---|
| `bind` | `127.0.0.1:30000` | Listen address |
| `db` | `./shadowcat.db` | SQLite database file |
| `config` *(CLI only)* | `shadowcat.toml` | TOML config path |
| `admin_user` / `admin_password` | *(unset)* | Headless first-admin bootstrap |
| `setup_token` | `auto` | Setup-form policy (table above) |
| `session_key` | *(generated + DB-persisted)* | Session-cookie signing key. Unset, a key is generated once and stored in the database, so sessions survive restarts by default; set it explicitly only for multi-replica setups or controlled key rotation |
| `assets_dir` | `assets/` beside the db | Uploaded-file store |
| `modules_dir` | `modules/` beside the db | Installed community modules |
| `log_dir` | `logs/` beside the db | Rolling daily log-file output root (retains 14 files; stdout logging is unaffected) |
| `backups_dir` | `backups/` beside the db | In-app backup output root |
| `upload_max_bytes` | 25 MiB | Upload size cap (players) |
| `upload_rate_per_min` | 20 | Uploads per minute (players) |
| `upload_max_bytes_gm` / `upload_rate_per_min_gm` | 2× player values | GM upload caps |
| `retain_originals` | `true` | Keep each converted upload's original bytes (`<uuid>.orig`) for reconvert/download; `false` saves disk |
| `login_per_min_per_identity` / `login_per_min_per_ip` | built-in | `/api/login` throttle budgets |
| `invite_per_min_per_account` / `invite_per_min_per_ip` | built-in | Invite-accept throttle budgets |
| `trusted_proxies` | *(empty)* | Reverse-proxy IP addresses trusted to set `X-Forwarded-For` and `X-Forwarded-Proto`; every entry must be an IP address (anything else stops the server at start, naming the entry) |
| `behind_proxy` | `false` | Declares a reverse proxy fronts this bind on the same host; makes a loopback bind count as exposed for the setup-token policy. Replaces `behind_tls_proxy`, which now stops the server at start |
| `tls` | `auto` | Built-in HTTPS: `auto` serves TLS on the same port beside plain HTTP, `off` serves plain HTTP only |
| `tls_cert` / `tls_key` | *(unset)* | Your own certificate chain and private key (PEM); both or neither |
| `tls_hostnames` | *(empty)* | Extra DNS names / IP addresses the self-signed certificate must cover |
| `tls_advertise_hostname` | `false` | List this machine's hostname (and `<hostname>.local`) in the self-signed certificate. Off by default because the certificate is shown to anyone who connects, before any login, and machine names often contain a person's name. Turn it on if players type the machine name; the price is that the name is visible to every peer that reaches the port. Changing it regenerates the certificate at the next start (browsers warn once more) |
| `tls_dir` | `tls/` beside the db | Where the self-signed certificate and key are kept |
| `public_url` | *(unset)* | The `https://` origin players should use (scheme, host and optional port only: user information, a path, a query or a fragment stop the server at start), published in `GET /api/config` |
| `egress_window_secs` | `60` | Seconds a peer may make no progress receiving data before the server cuts it (1 to 3600; anything else stops the server at start). Applies to every response and to each pending WebSocket send; see [Slow readers](#slow-readers) |
| `hsts_max_age_secs` | *(unset: no HSTS)* | Opt-in `Strict-Transport-Security` (read the HSTS section first) |
| `acme_domains`, `acme_email`, `acme_terms_agreed`, `acme_challenge`, `acme_directory`, `acme_ca_root` | *(unset)* | Accepted and validated; certificate issuance is not available in this version |

Logging: `SHADOWCAT_LOG` (falling back to `RUST_LOG`, then `info`) — e.g.
`SHADOWCAT_LOG=debug`.

## HTTPS and plain HTTP on one port

Shadowcat serves **both** `https://` and plain `http://` on the same port. The first byte of each
connection decides which one it is, so there is nothing to configure to get HTTPS and no redirect
ever moves a player from one to the other. **Plain HTTP works from any address, always** — a
TLS problem of any kind (a bad certificate file, an unwritable certificate directory) is logged and
reported, and never stops plain HTTP from serving.

On start the log lists every address:

```text
https://192.168.1.20:30000  (encrypted, browser warns once)
http://192.168.1.20:30000  (unencrypted, no warning)
```

### The default certificate

With `tls = "auto"` (the default) Shadowcat creates one self-signed certificate the first time it
runs and reuses it on every later start. It is stored in `tls/` beside the database file (`tls_dir`
moves it; an in-memory database keeps the certificate in memory for the life of the process, so it
changes on every start). The private key file is readable by its owner only (mode `0600`
on macOS/Linux; an owner-only access list on Windows). Shadowcat restricts the directory only when
it creates it (`0700` / owner-only access list); a directory you point `tls_dir` at that already
exists keeps its permissions. If the operating system refuses to restrict the key file, the key is
not written at all: the server logs it and serves an in-memory certificate for that run.

The certificate covers `localhost`, the loopback addresses, every entry of `tls_hostnames`, and the
LAN and global addresses of the machine's network adapters at the time it was generated (virtual
adapters such as Docker bridges and Hyper-V switches are skipped). It does **not** list the
machine's hostname unless you set `tls_advertise_hostname = true` (see the table above for why); a
browser then shows a name-mismatch error on top of the trust warning if a player types that name.
It is regenerated only when it expires within 30 days, when the file cannot be read, when a
`tls_hostnames` entry is missing from it, or when it lists the machine hostname while
`tls_advertise_hostname` is off (for example after you turn the option back off). A new network address does **not** regenerate it — that
would make every player accept the browser warning again after each DHCP change; the log notes the
mismatch once instead. Force a new certificate with:

```bash
./shadowcat --tls-regenerate
```

which regenerates, prints the new fingerprint and exits (it cannot be combined with `--backup-to`,
`--restore-from` or a subcommand).

**What players see.** A self-signed certificate is not vouched for by any authority, so each
browser shows a full-page "connection is not private" warning once per browser per certificate,
and again whenever the certificate changes. Clicking through gives an encrypted connection without
proof of who the server is. That is the reason plain HTTP stays fully supported: a player who does
not want the warning can use the `http://` address and simply has no encryption.

When the page is opened over plain HTTP from a non-loopback address and HTTPS is available, the
login screen offers the `https://` address in a dismissible notice (the dismissal is remembered per
address). It never blocks the login.

**Plain HTTP is unencrypted.** The login cookie, the password typed into the login form and all
game traffic cross the network in the clear. On a home network you trust that is usually
acceptable; for play over the internet, use HTTPS.

The certificate fingerprints (the served certificate's and the self-signed fallback's) are logged at
start and whenever a certificate is regenerated or reloaded. The fingerprint appears only in the server log and in the admin-only `GET
/api/admin/tls` response. It is deliberately not shown to players: a fingerprint fetched over the
channel being checked proves nothing.

### Your own certificate

```toml
tls_cert = "/etc/shadowcat/fullchain.pem"
tls_key = "/etc/shadowcat/privkey.pem"
```

Both keys or neither. The chain and key are checked at start (the key must match the leaf); the
files are re-read every 60 seconds so a renewed certificate takes effect without a restart, and a
failed re-read keeps the previous good certificate. A connection that names a host the certificate
covers gets this certificate; a connection by IP address, with no name, or for a name the
certificate does not cover gets the self-signed certificate that carries the addresses. If the
files cannot be used at boot, the server falls back to the self-signed certificate, logs why, and
reports `degraded_from: operator` on the admin route — it never refuses to start over a TLS file.

`public_url = "https://vtt.example:30000"` publishes the address players should use (`GET
/api/config`, `tls.https_origins`). The login notice offers a published address whose host the
player is already using; otherwise it offers the published addresses; with none published it offers
the address the player is on, switched to `https://`.

### Turning it off

`tls = "off"` serves plain HTTP only; a client that opens a TLS connection is dropped. Nothing else
changes.

### HSTS (opt-in, read the warning)

`hsts_max_age_secs = 31536000` adds `Strict-Transport-Security` to responses. HSTS is remembered
per **host name**, not per port: a browser that has seen it will refuse to use plain `http://` for
that host for the whole `max-age`, which ends plain-HTTP play for that browser, and on a
self-signed certificate turns the bypassable warning into a hard error. It is sent only when the
client-facing connection is TLS **and** either a trusted proxy fronts the server or the connection
was served the operator certificate for a name it covers; a connection served the self-signed
certificate never receives it. It is off by default, without `includeSubDomains` or `preload`.

### One session per scheme

The session cookie is `sc_sid` over plain HTTP and `__Host-sc_sid` (with `Secure`, `HttpOnly`,
`SameSite=Lax`, `Path=/`) over HTTPS. A browser therefore holds one login per scheme and switching
between `http://` and `https://` means logging in again. Browser storage (UI preferences kept in
`localStorage`) is also separate per origin. Upgrading a server that used the previous cookie
name logs every player out once.

### Microphone and other secure-context features

Browsers only allow microphone capture (the ducking module's voice-activity source), and some other
APIs, on a secure context: an `https://` page or `localhost`. Over plain HTTP from a LAN address the
module reports that the feature needs a secure connection and, when the server answers HTTPS, names
the `https://` address to open.

### Behind a reverse proxy

If a reverse proxy terminates TLS in front of Shadowcat, keep Shadowcat on loopback and set
`behind_proxy = true` (the setting was called `behind_tls_proxy`; the old name now stops the server
at start with a message, in the environment, the TOML file and on the command line, so a proxied
loopback deployment is never mistaken for a local-only one — delete the old key even if its value
was `false`). `behind_proxy` makes a loopback bind count as exposed for the setup-token policy.

The per-IP login/invite throttle keys off the raw TCP peer address, which behind a proxy is the
proxy itself. Setting `trusted_proxies` to the proxy's address makes the server trust that peer's
`X-Forwarded-For` (real client address) and `X-Forwarded-Proto` (whether the client used HTTPS,
which selects the `Secure` cookie flavour and the HSTS decision). Entries are matched by exact IP
address, never a CIDR range. A request whose immediate peer is not listed has both headers ignored.
The server reads the rightmost value of the last `X-Forwarded-Proto` line, so configure the proxy
to **set** the header rather than append to whatever the client sent. Setting `behind_proxy`
with an empty `trusted_proxies` logs a warning, because the scheme and client address then cannot
be learned.

`trusted_proxies` is a list, so the env-var form needs bracket syntax even for one entry:
`SHADOWCAT_TRUSTED_PROXIES=[127.0.0.1]` (a bare `SHADOWCAT_TRUSTED_PROXIES=127.0.0.1` fails to
parse). In `shadowcat.toml`, use ordinary TOML array syntax: `trusted_proxies = ["127.0.0.1"]`.
Loopback peers (`127.0.0.0/8`, `::1`) and peers listed in `trusted_proxies` are exempt from the
per-address connection cap that protects the listener (every player behind the proxy shares that
address); the global cap still applies to them. The exemption is not trust: a proxy or tunnel on the
same host (cloudflared, tailscale, ngrok, nginx) is exempt from the caps because it connects from
loopback, but it still has to be listed in `trusted_proxies` for the server to believe the
`X-Forwarded-For` and `X-Forwarded-Proto` headers it sends, which carry the real client address and
scheme.

### Automatic certificates (ACME)

The `acme_*` keys (`acme_domains`, `acme_email`, `acme_terms_agreed`, `acme_challenge`,
`acme_directory`, `acme_ca_root`) are accepted and their values are validated at start, but
certificate issuance is **not available in this version**. With `acme_domains` set the server logs
that ACME is disabled, serves the self-signed certificate, and reports `degraded_from: acme` with
the reason on the admin route. Use `tls_cert`/`tls_key` (for example from a certificate you renew
with your own tooling) until issuance ships.

### Browsers that upgrade `http://` to HTTPS

Some browsers can rewrite a typed `http://` address to `https://`. Because this server answers TLS
on the same port, an upgrade reaches the self-signed certificate and lands on the browser's
certificate warning, which can be clicked through, or the browser falls back to `http://`. The
table records what was measured and how. Engines behave differently per operating system, so a row
states the OS it was measured on; the CI matrix runs these rows on Windows, Linux and macOS, and
the test suite fails on CI for a row with no measurement on that OS (reporting what it observed), so
the rows are completed from those runs. Every row was measured with a private (RFC 1918) address; a
public or shared-address-space (`100.64.0.0/10`) address is upgraded by browsers like any internet
host, so on such an address the tests skip with a message locally and fail on CI.

| Browser and setting | Typed `http://<LAN address>` | How verified |
|---|---|---|
| Chromium, default settings | Loads over `http://` (no upgrade of an explicit `http://` address) | Automated (Playwright), asserted; measured on Windows |
| Chromium, "Always use secure connections" | Upgrade attempted, fails on the certificate, navigation blocked (`ERR_BLOCKED_BY_CLIENT`). The document shown is empty to automation, so its "continue" control cannot be reached; what a person sees is unverified | Attempt and block automated (net log), measured on Windows; the "continue" click was tried (keyboard and coordinate clicks) and is not drivable |
| Chromium, direct `https://` (certificate error page) | Certificate interstitial; "Advanced" then "Proceed" reaches the app | Automated, including the click; measured on Windows |
| Firefox, HTTPS-Only Mode (default private-address exemption) | Loads over `http://` | Automated (Playwright), asserted; measured on Windows |
| Firefox, HTTPS-Only Mode, a DNS name | Upgrade attempted, certificate error page; proceeding reaches the app | Error automated and asserted (measured on Windows), proceeding approximated by ignoring certificate errors |
| Firefox, HTTPS-Only Mode with `upgrade_local` on | Same as the DNS-name row | Automated (Playwright), asserted; measured on Windows |
| WebKit (Playwright build), default | Loads over `http://` | Automated (Playwright), asserted; measured on Windows |
| Safari on macOS or iOS, Chrome and Firefox on Android, any browser on a public IP address | — | Not run: UNVERIFIED |

The strict modes (Chromium's "Always use secure connections", Firefox with `upgrade_local`) are
non-default settings; players who enable them get the certificate warning on every `http://`
LAN address and can click through it, or set `tls = "off"` on the server.

### What has not been verified on real devices

`wss://` behaviour on iOS Safari and on Android Chrome after accepting the self-signed exception,
and the browser-upgrade behaviour of real (non-automated) browsers, have not been run on physical
devices: **UNVERIFIED**. The client shows a hint after three failed WebSocket connection attempts on
an `https://` page that uses the self-signed certificate, pointing to the same server's `http://`
address.

### Voice ducking from an HTTPS page

An `https://` page connecting to the ducking module's local monitor (`ws://127.0.0.1:<port>`) is a
mixed-content connection. Chromium and Firefox allow it for loopback (measured on Windows); the
WebKit build measured here (Playwright's) refuses it. Real Safari has not been tested. In such a browser the OS-monitor
source cannot connect from an `https://` page — open the server over plain `http://` or use another
browser for that feature. The settings section shows this hint on `https://` pages while the
monitor is not connected.

## Worlds and players

Two orthogonal role systems, worth keeping straight:

- **Server tier**: `admin` or `user`. Admins manage accounts and can create
  worlds. No world role grants server-admin.
- **World role**: `gm` or `player`, per world. The GM owns the world's rules,
  scenes, and module enablement.

The flow for seating a table:

1. **Accounts are admin-created** — there is no open registration. As admin,
   create accounts for your players (Settings → user management, or
   `POST /api/users`).
2. **Create a world**; its creator (or assignee) is GM.
3. **Invite**: the GM issues invite codes for the world; a logged-in player
   redeems a code and is seated with the `player` role. Invite acceptance is
   rate-limited server-side.

## Data on disk

Everything lives beside the database file unless configured elsewhere:

```
/srv/shadowcat/
├── shadowcat.db     the world state (single SQLite file)
├── assets/          uploaded images/files (content-addressed by UUID)
├── modules/         installed community modules (one folder per module)
├── tls/             the self-signed certificate and its private key (created on first start)
└── backups/         in-app backup output
```

All paths are OS-portable; on Windows the same layout appears wherever you point
`--db`. The modules folder may simply not exist — that scans as "no modules
installed".

## Backup and restore

The binary doubles as its own backup tool — both modes run **one-shot and exit**
without starting the server:

```bash
# Snapshot db + assets into a directory (refuses a non-empty dir without --force)
./shadowcat --db /srv/shadowcat/shadowcat.db --backup-to /mnt/backups/2026-07-30

# Restore a prior snapshot over the configured db + assets
./shadowcat --db /srv/shadowcat/shadowcat.db --restore-from /mnt/backups/2026-07-30 --force
```

`--backup-to` and `--restore-from` are mutually exclusive. The backup is a
consistent SQLite `VACUUM INTO` snapshot plus the asset files; on success it
prints one summary line — output directory, asset file count, database bytes,
and the Shadowcat version that wrote it (useful when restoring later). A running
server also writes backups into `backups_dir` through the admin-only
`POST /api/admin/backup` endpoint.

## Serving to your table

**LAN**: bind beyond loopback — `--bind 0.0.0.0:30000` — and open TCP 30000 in
the OS firewall (Windows Defender prompts on first listen; on Linux
`ufw allow 30000/tcp`; macOS System Settings → Firewall). Players connect to
`http://<your-lan-ip>:30000/` (no warning, unencrypted) or
`https://<your-lan-ip>:30000/` (encrypted, one certificate warning per browser); both are printed in
the log at start. Remember: a non-loopback bind flips the default setup policy to token-required —
read the token from the log on first boot.

**Internet**: forward the external port to the Shadowcat port and use the `https://` address, with
your own certificate for a public name (see above) or accepting the certificate warning. A
reverse proxy in front of a loopback bind also works; the client runs over a WebSocket at `/ws`,
so **upgrade headers must be forwarded**, and the proxy must tell Shadowcat which scheme the
player used:

```nginx
# In the http block. The Connection header is "upgrade" for a WebSocket handshake and empty
# (not sent) for every other request, so pooled upstream connections can be reused.
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      '';
}

server {
    listen 443 ssl;
    server_name vtt.example;

    location / {
        proxy_pass http://127.0.0.1:30000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

with `behind_proxy = true` and `trusted_proxies = ["127.0.0.1"]` in Shadowcat's configuration.

Caddy proxies WebSocket connections itself, and by default sets `X-Forwarded-For`,
`X-Forwarded-Proto` and `X-Forwarded-Host` while ignoring any values a client sent (unless the
client is listed in its `trusted_proxies`; do not list clients), per the
[`reverse_proxy` reference](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy). So the
whole site is:

```caddyfile
vtt.example {
    reverse_proxy 127.0.0.1:30000
}
```

Traefik adds `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host` by default
([FAQ](https://doc.traefik.io/traefik/getting-started/faq/)), and trusts forwarded headers a client
sends only from the addresses in the entry point's `forwardedHeaders.trustedIPs` (or when
`forwardedHeaders.insecure` is on; leave both unset for a public entry point), per its
[entry point reference](https://doc.traefik.io/traefik/reference/install-configuration/entrypoints/).
A file-provider router and service pointing at Shadowcat:

```yaml
http:
  routers:
    shadowcat:
      rule: Host(`vtt.example`)
      entryPoints: [websecure]
      service: shadowcat
      tls: {}
  services:
    shadowcat:
      loadBalancer:
        servers:
          - url: http://127.0.0.1:30000
```

In both, Shadowcat's `trusted_proxies` must list the address the proxy connects from (`127.0.0.1`
when it runs on the same host). HAProxy and other proxies need the equivalent: forward
`Upgrade`/`Connection` for the WebSocket, and send both `X-Forwarded-For` and `X-Forwarded-Proto`,
overwriting rather than appending to any value the client supplied; check your proxy's documentation
for its exact setting.

Shadowcat speaks HTTP/1.1 on plain connections (HTTP/2 without TLS, "h2c", is not supported), so
a proxy must use HTTP/1.1 to reach a plain upstream; over TLS it speaks what the client negotiates
(`h2` or `http/1.1`).

Shadowcat closes an HTTP/1.1 connection that sends no request for 30 seconds, an HTTP/2
connection that has had no request in flight for 60 seconds (a live WebSocket counts as a request
in flight), and an HTTP/2 connection that stops answering pings. It also caps live connections at
2048 in total and 256 per client address; loopback peers and addresses listed in `trusted_proxies`
are exempt from the per-address cap (the 2048 total still applies to them), so a reverse proxy or
tunnel on the same host is never refused by it, and one on another host is exempt once listed in
`trusted_proxies`, which it needs anyway for the real client address and scheme. A
proxy that pools idle upstream connections must drop them sooner than that, or it will try to
reuse a connection Shadowcat has already closed. For
nginx, an `upstream` block that uses `keepalive` holds an idle upstream connection for
`keepalive_timeout`, which defaults to 60 seconds
([`ngx_http_upstream_module`](https://nginx.org/en/docs/http/ngx_http_upstream_module.html)), so
set it below 30:

```nginx
upstream shadowcat {
    server 127.0.0.1:30000;
    keepalive 16;
    keepalive_timeout 20s;
}
```

(use `proxy_pass http://shadowcat;` with the `proxy_http_version 1.1;` and
`proxy_set_header Connection $connection_upgrade;` lines of the sample above). nginx's upstream
`keepalive` documentation requires the `Connection` request header to be cleared for pooled
HTTP/1.1 connections, and its `proxy_set_header` documentation says a header whose value is empty
is not passed on, which is why the `map` above yields an empty value for requests that are not
upgrades. The `map` in nginx's own WebSocket guide yields `close` there instead, which makes nginx
close the upstream connection after every ordinary request and leaves the `keepalive` pool unused.
WebSocket connections are unaffected by Shadowcat's idle bounds: the HTTP/1.1 bound applies to the
wait for a request head, not to an upgraded connection, and an HTTP/2 connection carrying a live
WebSocket is not idle.

### Connection caps and open-file limits

When a cap refuses a connection, the server logs one warning per 10 seconds that names the cap
(pre-hand-off or live connections), how many connections were refused since the last warning and
one refused client. A client is an IPv4 address or an IPv6 /64 prefix, the same unit the
per-address cap counts. Nothing is logged per refused connection.

Every connection holds one open file descriptor, so the caps only mean something if the process may
hold that many. On Linux and macOS the server raises its soft open-file limit at startup to what the
default caps and the reserve below need, never above the hard limit (on macOS no higher than the
system's `kern.maxfilesperproc`); a limit that is already enough is left alone. It then reads the
limit back and fits both caps inside that limit minus the reserve: 512 descriptors kept for the
database, logs and outbound requests, 128 for connections waiting to be handed to the server, and
256 for asset files held open while a slow client downloads them. The caps keep their ratio. If the
limit is too low to back the default caps (the common default of 1024 is), the caps are reduced and
a startup warning says to what; raise
the limit with `ulimit -n` or, under systemd, `LimitNOFILE=`, to restore them. Windows has no
comparable per-process limit and the caps are not reduced there.

The 256 descriptors for asset files are a real limit, enforced by the server: every asset response
that streams a file (audio, video, images, PDFs, archives, the retained-original download and the
other asset variants) and every world export holds one of 256 permits for as long as it holds the
file open. A permit comes back when the last byte of the file has been read from disk (even if the
client has not yet taken every byte) or when the response is cut or the client goes away. A request
that finds all 256 permits taken waits up to 2 seconds for one and is then refused with
`503 Service Unavailable` and `Retry-After: 1`, before any response header is written. Loading a
scene of 150 distinct images held at most 4 permits at once over both HTTP/1.1 and HTTP/2 (measured
with a debug-build gauge), so 256 is far above ordinary use; a table whose members stall many
downloads at once can exhaust it for up to the slow-reader window below.

### Slow readers

A response whose receiver makes no progress for `egress_window_secs` seconds is cut, at most a
quarter of that window late (so within 75 seconds at the default 60). A response whose receiver
returns no flow-control credit for that long, or whose socket accepts almost nothing, is cut: an
HTTP/2 stream is reset and the rest of the connection is untouched, or, when the response had
already ended and its last bytes are stuck, or over HTTP/1, the whole connection is closed. A
WebSocket send that stays pending for that long ends its session; over HTTP/2 a session that is
more than a send buffer (64 KiB) ahead of what its peer has taken stops sending until the peer
catches up, and that wait counts. A connection with nothing in flight is never cut by this setting.
A cut asset response of a type served with byte-range support (audio, video, images, PDFs and
archives) can be resumed with a `Range` request from the first byte not received; a cut world
export has no length and no `Range` and starts again from the beginning, and so does a cut
response of a compressible type such as text or SVG.

A socket takes data in lumps rather than steadily. On Windows loopback the lumps measured were
about 100 to 520 KB, so a reader that consumes less than about one lump per window (roughly 2 to 9
KB/s at the 60 second default) can be cut while it is still reading. Real network paths were not
measured; their steps are expected to be finer. Over an extremely slow network, raise
`egress_window_secs`. For an HTTP/2 stream whose receiver returns flow-control credit slowly the
measured boundary depends on the window: at a 1 second window the outcome was mixed between about
18 and 33 KiB/s, and at a 3 second window it matched a one-chunk-per-window model. Those
measurements used the `h2` crate as the client; a browser's own windows and credit batching were
not measured. Measured with the same client, a reader at or below about 18 KiB/s was always cut at
the 1 second window and one at or above about 40 KiB/s never was; the 60 second default was not
measured, and raising the setting lowers the rate that is cut.

A browser that cancels many requests at once (navigating away from a page of images) is served
through however many cancellations arrive together; a connection that keeps cancelling streams
faster than about 1,600 a second for ten seconds is ended as an attack.

### Shutdown

On Ctrl+C or SIGTERM the server tells every WebSocket session to close, then gives open
connections 5 seconds to finish. A session whose client has stopped reading is given 2 seconds to
send its close message and is then dropped; a send that stays pending for `egress_window_secs` (60
by default) ends its session, so a client that stops reading while connected is dropped on its
own. Connections still open
after the 5 seconds are closed, and WebSocket sessions get a further 3 seconds at least to finish,
so the server exits within about 8 seconds.

A request that is still running at that point is cut off, not finished. The operations that can be
running that long leave recoverable state: a chunked upload is lost with the process, the client
starts it again, and its partly written `*.tmp` file stays in the world's asset folder; an asset
replace commits its database row before it swaps the file, so a cut replace leaves at most one
stray `*.tmp` file beside the asset (the row is then one version ahead of unchanged bytes and
clients fetch it again); such `*.tmp` files can be deleted; a backup that is cut off leaves a backup
folder without `manifest.json`, which restore refuses, and which can be deleted; a world export
only reads.

## Mobile

Players join from phone browsers — the client is responsive and touch-ready, and
there is nothing to install. The same URL serves desktop and mobile.

## Voice ducking on your machine

The ducking module's Settings section can duck the table's music/ambience while Discord (or
any voice app) is talking, without the Discord SDK — it reads OS-reported audio-session peak
levels through a small companion command:

```bash
shadowcat audio-monitor --port 31998 --allow-origin http://localhost:30000 --watch discord
```

The Settings section prints this exact command line (with `--allow-origin` set to the page's
own origin, so an `https://` page prints an `https://` origin) — copy it verbatim rather than
retyping it. Without `--allow-origin` the monitor admits `http://` and `https://` on `localhost` and
`127.0.0.1` at port 30000. It runs entirely on your own machine,
never uploads anything, and only ever reports the peak level of processes matching `--watch`
(default `discord`) — every other running application stays invisible to it and to the page.

**Windows**: no extra setup; WASAPI session metering works out of the box.

**macOS 14.2+**: the first run triggers the OS's own "System Audio Recording" permission
prompt — accept it once. Older macOS reports itself unsupported honestly rather than failing
silently.

**Linux**: needs a running PipeWire session (the default on current Fedora/Ubuntu desktops);
without one the monitor reports "PipeWire not running" rather than crashing.

## Troubleshooting

- **"address already in use"** — another process holds the port; change `--bind`
  or stop the other process.
- **"database is locked"** — a second Shadowcat process is pointed at the same
  db file. One server per database: the pool is deliberately single-connection.
- **Installed module doesn't appear** — the folder name under `modules/` is the
  module's identity; the manifest must declare `engines.shadowcat` matching the
  server version, or enable is refused. Check the world's Installed-modules
  panel for the rejection reason.
- **Backup refuses to run** — `--backup-to` into a non-empty directory needs
  `--force` (protection against clobbering a previous snapshot).
- **Setup form asks for a token you don't have** — it's in the server log from
  the first boot on a non-loopback bind (`setup token required; ...`).
