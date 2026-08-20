# Hosting a Shadowcat server

Shadowcat ships as **one native executable**: the Rust server with the entire web
client embedded inside it. Hosting a game means running that one binary and
pointing browsers at it — no database server, no web server, no separate client
deploy.

## Get the binary

**From a release:** download the artifact for your OS — a macOS `.app` bundle, a
Linux staging tree with a `.desktop` entry, or a Windows `shadowcat.exe` (icon
embedded).

**From source** (needs Rust stable, Node 22, pnpm 9):

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
`127.0.0.1:30000`. Open `http://localhost:30000/` in a browser.

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
| `backups_dir` | `backups/` beside the db | In-app backup output root |
| `upload_max_bytes` | 25 MiB | Upload size cap (players) |
| `upload_rate_per_min` | 20 | Uploads per minute (players) |
| `upload_max_bytes_gm` / `upload_rate_per_min_gm` | 2× player values | GM upload caps |
| `login_per_min_per_identity` / `login_per_min_per_ip` | built-in | `/api/login` throttle budgets |
| `invite_per_min_per_account` / `invite_per_min_per_ip` | built-in | Invite-accept throttle budgets |
| `trusted_proxies` | *(empty)* | Reverse-proxy IPs trusted to set `X-Forwarded-For` |

Logging: `SHADOWCAT_LOG` (falling back to `RUST_LOG`, then `info`) — e.g.
`SHADOWCAT_LOG=debug`.

### Behind a reverse proxy

By default the per-IP login/invite throttle keys off the raw TCP peer address. Behind a reverse
proxy that terminates the client connection, that peer is always the proxy itself, so every real
client collapses into one shared throttle bucket. Setting `trusted_proxies` to the proxy's address
(e.g. `SHADOWCAT_TRUSTED_PROXIES=127.0.0.1` for a reverse proxy on the same host) makes the server
trust that peer's `X-Forwarded-For` header and resolve the real client address from it instead.
Entries are matched by exact IP address only, never a CIDR range — list every trusted proxy
explicitly. A request whose immediate TCP peer is not in this list has its `X-Forwarded-For` header
ignored entirely, so an untrusted client cannot spoof its own address by sending the header
directly.

`trusted_proxies` is a list, so the env-var form needs bracket syntax even for one entry:
`SHADOWCAT_TRUSTED_PROXIES=[127.0.0.1]` (a bare `SHADOWCAT_TRUSTED_PROXIES=127.0.0.1` fails to
parse). In `shadowcat.toml`, use ordinary TOML array syntax: `trusted_proxies = ["127.0.0.1"]`.

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
server can also produce backups into `backups_dir` from the admin UI.

## Serving to your table

**LAN**: bind beyond loopback — `--bind 0.0.0.0:30000` — and open TCP 30000 in
the OS firewall (Windows Defender prompts on first listen; on Linux
`ufw allow 30000/tcp`; macOS System Settings → Firewall). Players connect to
`http://<your-lan-ip>:30000/`. Remember: a non-loopback bind flips the default
setup policy to token-required — read the token from the log on first boot.

**Internet / reverse proxy**: keep Shadowcat on loopback and put a TLS proxy in
front. The one thing proxies get wrong: the client runs over a WebSocket at
`/ws`, so **upgrade headers must be forwarded**:

```nginx
server {
    listen 443 ssl;
    server_name vtt.example;

    location / {
        proxy_pass http://127.0.0.1:30000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
    }
}
```

Terminate HTTPS at the proxy; Shadowcat itself speaks plain HTTP.

## Mobile

Players join from phone browsers — the client is responsive and touch-ready, and
there is nothing to install. The same URL serves desktop and mobile.

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
