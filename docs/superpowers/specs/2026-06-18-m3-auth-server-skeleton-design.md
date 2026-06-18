# M3 — Auth + Server Skeleton: Design

Status: approved (brainstorm). Date: 2026-06-18.
Roadmap: [`docs/PLAN.md`](../../PLAN.md) M3. Architecture source of truth: [`docs/design/ARCHITECTURE.md`](../../design/ARCHITECTURE.md).

## 1. Goal

Stand up the authoritative HTTP server process: it boots, runs migrations, authenticates admin-provisioned accounts with argon2 + DB-backed sessions, formalizes the server-level role, exposes `/health`, emits structured logs with request ids, and ships as a single binary with the client bundle embedded. First account creation happens through a first-run web setup flow, with a headless override for remote hosting.

## 2. Scope & non-goals

**In scope**
- axum process that boots and runs migrations on startup.
- argon2 password hashing; tower-sessions DB-backed sessions.
- A formalized `ServerRole { Admin, User }` (the server tier), alongside the existing `WorldRole { Gm, Player, Spectator }` and `DocRole { Owner, Observer, None }`.
- First-run web setup flow + headless env/CLI bootstrap.
- JSON API: `GET /health`, `GET /api/me`, `POST /api/setup`, `POST /api/login`, `POST /api/logout`.
- Structured logging (tracing) + per-request request ids.
- Single-binary build: stub bundle + transitional static auth pages embedded via rust-embed.
- Layered configuration (CLI > env > TOML file > defaults).

**Out of scope (and why)**
- **WebSocket event bus** — M4.
- **Document CRUD over HTTP** — M5. Because no untrusted document write path is exposed in M3, the deferred `validation::validate_system_size` / `validate_field_path` wiring (see `docs/TODO.md`) **stays deferred to M5**; M3 does not pull it in.
- **Svelte client UI** — ~M7. M3 ships transitional hand-authored static auth pages, not the real UI.
- **Self-registration / email / password reset** — not in v1 (admin-provisioned accounts only, per ARCHITECTURE §3).

## 3. Crate & module layout

Stay a **single crate** (`shadowcat`). A Cargo workspace split is unjustified at this size; the seam to split later is just module boundaries, which this layout already provides.

New / changed files under `src/server/src/`:

```
main.rs                 # real entrypoint: #[tokio::main]
config.rs               # layered Config (figment + clap)
auth/
  mod.rs                # re-exports
  role.rs               # ServerRole
  password.rs           # argon2 hash/verify
  setup.rs              # first-run + headless bootstrap (shared create_admin)
  session.rs            # AuthUser / AdminUser extractors, session helpers
http/
  mod.rs                # router builder + AppState
  routes.rs             # handlers
  middleware.rs         # init-gate; request-id + trace wiring
  embed.rs              # rust-embed static asset serving
  error.rs              # AppError -> response mapping
```

New server-owned static directory (embedded, transitional):

```
src/server/static/
  index.html            # placeholder ("UI not yet built")
  setup.html            # first-run admin creation form
  login.html            # login form
  auth.js               # vanilla JS driving the JSON API
  styles.css
```

`src/server/src/health.rs`, `db.rs`, and `data/` are unchanged except `data` gains `ServerRole` use and an updated `create_user` signature (see §5).

## 4. Dependencies

Added to `src/server/Cargo.toml` (all MIT/Apache-2.0 — satisfies the permissive-license invariant, ARCHITECTURE §2.9):

| Crate | Purpose |
|---|---|
| `axum` 0.8 | HTTP routing + handlers |
| `tower` 0.5 | middleware composition |
| `tower-http` (features: `trace`, `request-id`) | request-id + tracing layers |
| `tower-sessions` 0.13 | session management layer |
| `tower-sessions-sqlx-store` (sqlite) | DB-backed session store |
| `argon2` 0.5 | Argon2id password hashing |
| `tracing`, `tracing-subscriber` (env-filter) | structured logging |
| `rust-embed` 8 | embed static bundle into the binary |
| `clap` 4 (derive) | CLI flags |
| `figment` 0.10 (toml + env providers) | layered config |
| `mime_guess` | content-type for embedded assets |

Dev-dependencies: `reqwest` (or `axum-test`) for in-process integration tests. (`tempfile` already present.)

**Verify during planning:** the repo is on **sqlx 0.9**. `tower-sessions-sqlx-store` must support that sqlx major. If no compatible release exists, the fallback (in priority order) is: pin a compatible store version → use a different DB-backed `SessionStore` → implement a thin custom `SessionStore` over the existing pool. This is a version-compatibility check, not a design change.

## 5. Schema & roles

### Migration `migrations/0002_auth.sql`
```sql
ALTER TABLE users ADD COLUMN password_hash TEXT;  -- nullable

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```
- `password_hash` is **nullable**: M2's credential-less data-layer test users remain valid rows but cannot authenticate (login requires a non-null hash).
- `settings` holds the persisted session signing key (key `session_key`) and may hold a `setup_complete` marker.
- The **session table is created separately** by `tower-sessions-sqlx-store`'s own migration, invoked once at startup — it is not part of `sqlx::migrate!`.

### Roles
- New `ServerRole { Admin, User }` in `auth/role.rs`, serde `rename_all = "snake_case"` → `"admin"` / `"user"`. This replaces the free-form `server_role: &str`.
- `SqliteRepository::create_user` signature changes to accept a `ServerRole` and an `Option<&str>` password hash; M2's data-layer tests update their call sites (tests yield to correct code).
- `WorldRole` and `DocRole` are unchanged. `ServerRole` is the **server tier** (admin vs ordinary user); world/document roles are orthogonal and already exist.

## 6. Authentication mechanics

- **Password hashing** (`auth/password.rs`): Argon2id at the crate's default params. `hash_password(plain) -> String` (PHC string), `verify_password(plain, phc) -> bool`. Unit-tested: verify true on match, false on mismatch, independent salts produce distinct hashes.
- **Sessions** (`auth/session.rs`): `tower-sessions` `SessionManagerLayer` backed by `SqliteStore`. Session data is server-side; the cookie carries only the session id. Cookie attributes: `HttpOnly`, `SameSite=Lax`, `Secure` when the bind address is non-loopback. On successful login, the user id and `ServerRole` are written into the session.
- **Session signing key**: generated once on first boot, persisted in `settings` (`session_key`), reused thereafter so sessions survive restart with no operator action. Overridable via config `session_key`.
- **Extractors**: `AuthUser` (any authenticated user) and `AdminUser` (server-role admin) implement `FromRequestParts`. Missing session → 401; insufficient role → 403.

## 7. First-run setup, token, and bootstrap

On boot the server checks whether any admin user exists → **uninitialized** or **normal** state.

### Uninitialized
- Init-gate middleware redirects every non-asset, non-`/api/setup` request to `/setup`.
- `POST /api/setup { username, password, token? }` creates the first admin via the shared `create_admin` routine, then the gate closes **permanently**. Subsequent `/api/setup` → 409 Conflict.

### Setup token
- Config `setup_token = auto | off | required | <explicit-string>` (default `auto`).
- `auto` ⇒ token **required iff the bind address is non-loopback** (e.g. `0.0.0.0` / public), **open** on `127.0.0.1` / `::1`.
- When a token is required and no explicit value was supplied, the server generates a random token at boot and prints it to stdout. `POST /api/setup` must echo the token, or it is rejected (403).
- `off` forces the window open regardless of bind; `required` forces it closed regardless of bind; an explicit string sets the token value directly.

### Headless bootstrap
- If `admin_user` + `admin_password` are present in config (CLI/env/file) and no admin exists, the server seeds the admin on boot via the same `create_admin` routine and **never opens `/setup`** (no token involved). This is the remote-hosting path.

All three entries (web setup, headless bootstrap) funnel through one `create_admin` routine — the single place that hashes the password and writes the admin user.

## 8. HTTP surface

| Method + path | Auth | Behavior |
|---|---|---|
| `GET /health` | none | `HealthStatus` JSON, now reporting real `db_connected`. |
| `GET /api/me` | session | `{ id, username, server_role }` or 401. |
| `POST /api/setup` | gated | `{ username, password, token? }` → create first admin; 409 once initialized. |
| `POST /api/login` | none | `{ username, password }` → argon2 verify → session; 401 on failure. |
| `POST /api/logout` | session | destroy session. |
| `GET /*` | none | embedded static assets, subject to the init-gate redirect. |

- **Login failure** returns a uniform 401 with a generic message (no user-enumeration: same response whether the username is unknown or the password is wrong).
- **State**: `AppState { repo: Arc<SqliteRepository>, config: Arc<Config> }`.
- **Middleware order** (outermost → innermost): `SetRequestId` → `Trace` → `SessionManager` → init-gate → routes.

## 9. Configuration

Layered via figment + clap, precedence **CLI flag > `SHADOWCAT_*` env var > TOML file > built-in default**.

| Key | CLI / env | Default |
|---|---|---|
| `bind` | `--bind` / `SHADOWCAT_BIND` | `127.0.0.1:30000` |
| `db` | `--db` / `SHADOWCAT_DB` | `./shadowcat.db` |
| `config` (file path) | `--config` / `SHADOWCAT_CONFIG` | `./shadowcat.toml` if present |
| `admin_user` | `--admin-user` / `SHADOWCAT_ADMIN_USER` | unset |
| `admin_password` | `--admin-password` / `SHADOWCAT_ADMIN_PASSWORD` | unset |
| `setup_token` | `--setup-token` / `SHADOWCAT_SETUP_TOKEN` | `auto` |
| `session_key` | `--session-key` / `SHADOWCAT_SESSION_KEY` | generated + persisted in `settings` |

The TOML file mirrors these keys. Missing file is not an error; a malformed file fails boot with a logged root cause.

## 10. Observability & error handling

- `tracing-subscriber` with an env filter (`RUST_LOG` / `SHADOWCAT_LOG`). `tower-http::trace` emits a per-request span; the request id (from `SetRequestId`, propagated) is a span field. Fields: method, path, status, latency.
- Handlers return `Result<T, AppError>`; `AppError` maps to clean status codes (401 / 403 / 409 / 422 / 500). 5xx responses are logged with the request id and never leak internal detail into the body.
- Boot failures (malformed config, migration error, port already in use) log the root cause and exit non-zero — the active-remediation posture in `CLAUDE.md`.

## 11. Single-binary embed

- `rust-embed` over `src/server/static/` for M3.
- **Documented seam**: once the Vite client bundle exists, the embed root moves to the client `dist/` output (or the build copies the client bundle into the embed root), and these transitional auth pages are replaced by the real Svelte auth UI. The embed module exposes a single function (`serve_embedded(path) -> Response`) so only its root changes later, not its callers.

## 12. Testing

**Unit**
- `password`: hash/verify true-on-match, false-on-mismatch, distinct salts.
- `role`: `ServerRole` serde round-trip (`"admin"`/`"user"`).
- `config`: precedence (CLI > env > file > default); `setup_token = auto` derives required-vs-open from a loopback vs non-loopback bind.

**Integration** (in-process axum + reqwest / axum-test, in-memory or tempfile SQLite)
- Setup creates the first admin, then `/api/setup` returns 409.
- Token enforced when bound non-loopback; open when bound loopback (`auto`).
- Login success sets a session; wrong password and unknown user both → 401 with identical body.
- Session cookie gates `GET /api/me`; absent/invalid session → 401.
- `AdminUser` guard rejects a non-admin session (403).
- `GET /health` reports `db_connected = true` against a live pool.
- Headless bootstrap (`admin_user`/`admin_password` set, no admin present) seeds the admin and leaves `/setup` closed.

## 13. Decisions locked in this brainstorm

1. First account via a **first-run web setup flow** (`/setup`), not out-of-band provisioning; **headless env/CLI bootstrap** as the remote-hosting override. Both funnel through one `create_admin`.
2. Setup-window protection: **token derived from bind address** (`auto`: required on non-loopback, open on loopback), with an explicit `off` / `required` / `<value>` override.
3. M3 auth UI: **minimal hand-authored static HTML + vanilla JS**, embedded, transitional — replaced by the real Svelte UI later.
4. Config: **layered CLI > env > TOML > defaults** (figment + clap).
5. **Single crate**, modules as in §3.
6. **Session signing key persisted in a `settings` table**, auto-generated, config-overridable.
7. Transitional auth HTML lives in **`src/server/static/`**.
