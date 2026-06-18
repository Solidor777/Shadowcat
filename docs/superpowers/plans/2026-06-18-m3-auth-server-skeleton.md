# M3 — Auth + Server Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project uses the `mainline-plan-execution` skill (per the operator's global workflow rule) to implement this plan task-by-task — it replaces `superpowers:subagent-driven-development` / `superpowers:executing-plans`. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the authoritative axum HTTP server: it boots, runs migrations, authenticates admin-provisioned accounts (argon2 + DB-backed sessions), serves `/health`, emits structured logs with request ids, and ships as a single binary with an embedded stub bundle — with first-account creation via a first-run web setup flow plus a headless bootstrap override.

**Architecture:** Single `shadowcat` crate. New `config`, `auth/*`, and `http/*` modules sit on top of the existing M2 `data` layer. The server is server-authoritative: the HTTP layer authenticates and gates, the `data::SqliteRepository` persists. First-run state is derived from "does any admin exist?"; a bind-derived setup token guards the open setup window; a headless config path seeds the admin without the browser.

**Tech Stack:** Rust 2021, tokio, axum 0.8, tower / tower-http, tower-sessions (+ sqlx sqlite store), argon2, tracing, rust-embed, clap, figment, sqlx 0.9 (sqlite), uuid, serde.

## Global Constraints

- **Source layout:** all server source under `src/server/src/`; migrations under `src/server/migrations/`; embedded static assets under `src/server/static/`. (ARCHITECTURE §1.)
- **Licenses:** every added dependency must be MIT / Apache-2.0 / BSD / zlib / MPL-2.0. No GPL/AGPL/SSPL/proprietary. (ARCHITECTURE §2.9.)
- **Server-authoritative:** clients are never trusted for state, visibility, or permissions. (ARCHITECTURE §2.1.)
- **ts-rs sync:** any `#[derive(TS)]` type change must regenerate the TS mirror under `src/types/generated/`; CI enforces sync. (Existing `health.rs` invariant.)
- **No semantic validation of the `system` body server-side** — out of scope here anyway (no document CRUD in M3).
- **Roles:** `ServerRole { Admin, User }` (server tier) is orthogonal to the existing `WorldRole { Gm, Player, Spectator }` and `DocRole { Owner, Observer, None }`; do not conflate them.
- **No self-registration / email / password reset** in v1.
- **No debug code in compiled builds:** diagnostics go through `tracing` / `debug_assert!`, never stray `println!`/`dbg!`. (CLAUDE.md.)
- **Secrets via config, never hardcoded;** tests use RFC-2606 / reserved synthetic values only. (CLAUDE.md.)
- **Defaults:** bind `127.0.0.1:30000`; db `./shadowcat.db`; `setup_token` policy `auto`.

## Version-compatibility check (do this before Task 6)

The repo is on **sqlx 0.9**. `tower-sessions-sqlx-store` must support that sqlx major. Before starting Task 6, run `cargo add tower-sessions tower-sessions-sqlx-store --dry-run` (or check docs.rs) and confirm a release pairs with sqlx 0.9 and a compatible `tower-sessions` version. If none exists, fall back in priority order: (a) pin a compatible store+tower-sessions pair; (b) use a different DB-backed `SessionStore`; (c) implement a thin custom `SessionStore` over the existing pool. Record the chosen versions in the Task 6 commit message. This is a version pin, not a design change.

## File Structure

| File | Responsibility |
|---|---|
| `src/server/Cargo.toml` | dependency additions (per-task) |
| `src/server/migrations/0002_auth.sql` | `users.password_hash` column + `settings` table |
| `src/server/src/lib.rs` | module declarations |
| `src/server/src/data/document.rs` | (unchanged enums; referenced) |
| `src/server/src/data/sqlite.rs` | `create_user` signature change; `user_by_username`, `admin_exists`, `get_setting`, `set_setting` |
| `src/server/src/auth/mod.rs` | re-exports |
| `src/server/src/auth/role.rs` | `ServerRole` |
| `src/server/src/auth/password.rs` | argon2 `hash_password` / `verify_password` |
| `src/server/src/auth/session.rs` | `SessionUser`, `AuthUser`/`AdminUser` extractors, session-key load/persist, session layer builder |
| `src/server/src/auth/setup.rs` | `create_admin`, `bootstrap_admin` |
| `src/server/src/config.rs` | `Cli`, `Config`, layered load, setup-token policy |
| `src/server/src/http/mod.rs` | `AppState`, `router()` |
| `src/server/src/http/routes.rs` | handlers (`health`, `me`, `login`, `logout`, `setup`) |
| `src/server/src/http/middleware.rs` | init-gate; request-id + trace wiring |
| `src/server/src/http/embed.rs` | rust-embed static serving |
| `src/server/src/http/error.rs` | `AppError` → response |
| `src/server/src/main.rs` | `#[tokio::main]` entrypoint |
| `src/server/static/{index,setup,login}.html`, `auth.js`, `styles.css` | transitional static auth UI |

---

### Task 1: Schema migration, `ServerRole`, and user/settings repository methods

**Files:**
- Create: `src/server/migrations/0002_auth.sql`
- Create: `src/server/src/auth/mod.rs`, `src/server/src/auth/role.rs`
- Modify: `src/server/src/lib.rs` (add `pub mod auth;`)
- Modify: `src/server/src/data/sqlite.rs` (`create_user` signature; new methods + tests; update M2 call sites)

**Interfaces:**
- Produces: `auth::role::ServerRole { Admin, User }` with `fn as_str(self) -> &'static str` and serde snake_case.
- Produces on `SqliteRepository`:
  - `async fn create_user(&self, username: &str, password_hash: Option<&str>, role: ServerRole, now: i64) -> Result<Uuid, DataError>`
  - `async fn user_by_username(&self, username: &str) -> Result<Option<UserRecord>, DataError>` where `pub struct UserRecord { pub id: Uuid, pub username: String, pub password_hash: Option<String>, pub server_role: ServerRole }`
  - `async fn admin_exists(&self) -> Result<bool, DataError>`
  - `async fn get_setting(&self, key: &str) -> Result<Option<String>, DataError>`
  - `async fn set_setting(&self, key: &str, value: &str) -> Result<(), DataError>`

- [ ] **Step 1: Write the migration**

Create `src/server/migrations/0002_auth.sql`:
```sql
ALTER TABLE users ADD COLUMN password_hash TEXT;

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

- [ ] **Step 2: Add the `ServerRole` enum + module**

Create `src/server/src/auth/role.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Server-tier role. Orthogonal to `WorldRole` (per-world) and `DocRole`
/// (per-document): this gates server-level administration only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerRole {
    Admin,
    User,
}

impl ServerRole {
    /// Stable storage token persisted in `users.server_role`.
    pub fn as_str(self) -> &'static str {
        match self {
            ServerRole::Admin => "admin",
            ServerRole::User => "user",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_role_serde_round_trips_snake_case() {
        assert_eq!(serde_json::to_value(ServerRole::Admin).unwrap(), serde_json::json!("admin"));
        let r: ServerRole = serde_json::from_value(serde_json::json!("user")).unwrap();
        assert_eq!(r, ServerRole::User);
        assert_eq!(ServerRole::Admin.as_str(), "admin");
    }
}
```

Create `src/server/src/auth/mod.rs`:
```rust
pub mod role;
```

Add to `src/server/src/lib.rs`:
```rust
pub mod auth;
```

- [ ] **Step 3: Run the role test to verify it passes**

Run: `cargo test -p shadowcat auth::role`
Expected: PASS (1 test).

- [ ] **Step 4: Write failing tests for the new repository methods**

Append to the `tests` module in `src/server/src/data/sqlite.rs`:
```rust
#[tokio::test]
async fn user_by_username_and_admin_exists() {
    use crate::auth::role::ServerRole;
    let r = repo().await;
    assert!(!r.admin_exists().await.unwrap());
    let id = r
        .create_user("admin1", Some("phc-hash"), ServerRole::Admin, 100)
        .await
        .unwrap();
    assert!(r.admin_exists().await.unwrap());
    let rec = r.user_by_username("admin1").await.unwrap().unwrap();
    assert_eq!(rec.id, id);
    assert_eq!(rec.server_role, ServerRole::Admin);
    assert_eq!(rec.password_hash.as_deref(), Some("phc-hash"));
    assert!(r.user_by_username("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn settings_get_set_round_trip() {
    let r = repo().await;
    assert!(r.get_setting("k").await.unwrap().is_none());
    r.set_setting("k", "v1").await.unwrap();
    assert_eq!(r.get_setting("k").await.unwrap().as_deref(), Some("v1"));
    r.set_setting("k", "v2").await.unwrap();
    assert_eq!(r.get_setting("k").await.unwrap().as_deref(), Some("v2"));
}
```

- [ ] **Step 5: Run to verify they fail**

Run: `cargo test -p shadowcat data::sqlite`
Expected: FAIL — `create_user` arity mismatch + `user_by_username`/`admin_exists`/`get_setting`/`set_setting`/`UserRecord` not found.

- [ ] **Step 6: Change `create_user` and add the new methods**

In `src/server/src/data/sqlite.rs`, add near the top:
```rust
use crate::auth::role::ServerRole;

/// Auth-facing projection of a user row.
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: Uuid,
    pub username: String,
    pub password_hash: Option<String>,
    pub server_role: ServerRole,
}
```

Replace the existing `create_user` with:
```rust
pub async fn create_user(
    &self,
    username: &str,
    password_hash: Option<&str>,
    role: ServerRole,
    now: i64,
) -> Result<Uuid, DataError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, server_role, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(now)
    .execute(&self.pool)
    .await?;
    Ok(id)
}

pub async fn user_by_username(&self, username: &str) -> Result<Option<UserRecord>, DataError> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, server_role FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(&self.pool)
    .await?;
    Ok(match row {
        Some(r) => {
            let role_str: String = r.get("server_role");
            let server_role = match role_str.as_str() {
                "admin" => ServerRole::Admin,
                _ => ServerRole::User,
            };
            Some(UserRecord {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                username: r.get("username"),
                password_hash: r.get("password_hash"),
                server_role,
            })
        }
        None => None,
    })
}

pub async fn admin_exists(&self) -> Result<bool, DataError> {
    let row = sqlx::query("SELECT 1 FROM users WHERE server_role = 'admin' LIMIT 1")
        .fetch_optional(&self.pool)
        .await?;
    Ok(row.is_some())
}

pub async fn get_setting(&self, key: &str) -> Result<Option<String>, DataError> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
    Ok(row.map(|r| r.get("value")))
}

pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), DataError> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 7: Update M2 call sites in the existing tests**

In `src/server/src/data/sqlite.rs` tests, replace every `create_user("author", "user", 0)` with `create_user("author", None, ServerRole::User, 0)` and `create_user("gm", "admin", 100)` with `create_user("gm", None, ServerRole::Admin, 100)`. The existing `members_carry_world_role` test's `create_user("gm", "admin", 100)` becomes `create_user("gm", None, ServerRole::Admin, 100)`. (Tests yield to correct code — CLAUDE.md.)

- [ ] **Step 8: Run the full data suite**

Run: `cargo test -p shadowcat data::`
Expected: PASS (all prior M2 tests + the 2 new ones).

- [ ] **Step 9: Commit**

```bash
git add src/server/migrations/0002_auth.sql src/server/src/auth/ src/server/src/lib.rs src/server/src/data/sqlite.rs
git commit -m "feat(m3): add auth schema, ServerRole, and user/settings repo methods"
```

---

### Task 2: Password hashing (argon2)

**Files:**
- Create: `src/server/src/auth/password.rs`
- Modify: `src/server/src/auth/mod.rs` (add `pub mod password;`)
- Modify: `src/server/Cargo.toml` (add `argon2`)

**Interfaces:**
- Produces: `auth::password::hash_password(plain: &str) -> Result<String, argon2::password_hash::Error>` (returns a PHC string) and `auth::password::verify_password(plain: &str, phc: &str) -> bool`.

- [ ] **Step 1: Add the dependency**

In `src/server/Cargo.toml` `[dependencies]`:
```toml
argon2 = { version = "0.5", features = ["std"] }
```

- [ ] **Step 2: Write the failing test**

Create `src/server/src/auth/password.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_true_on_match_false_on_mismatch() {
        let hash = hash_password("correct horse").expect("hash");
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong horse", &hash));
        assert!(!verify_password("correct horse", "not-a-phc-string"));
    }

    #[test]
    fn distinct_salts_produce_distinct_hashes() {
        let a = hash_password("same").expect("hash a");
        let b = hash_password("same").expect("hash b");
        assert_ne!(a, b, "random salt must make hashes differ");
        assert!(verify_password("same", &a));
        assert!(verify_password("same", &b));
    }
}
```

Add to `src/server/src/auth/mod.rs`:
```rust
pub mod password;
```

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p shadowcat auth::password`
Expected: FAIL — `hash_password` / `verify_password` not defined.

- [ ] **Step 4: Implement**

Prepend to `src/server/src/auth/password.rs` (above the test module):
```rust
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Hash a plaintext password with Argon2id (default params), returning a PHC
/// string that embeds the random salt. Source: Argon2 RFC 9106 via the `argon2` crate.
pub fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(plain.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored PHC string. Returns false on
/// any parse or mismatch error — callers must not distinguish the two.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat auth::password`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/auth/
git commit -m "feat(m3): add argon2 password hashing"
```

---

### Task 3: Layered configuration

**Files:**
- Create: `src/server/src/config.rs`
- Modify: `src/server/src/lib.rs` (add `pub mod config;`)
- Modify: `src/server/Cargo.toml` (add `clap`, `figment`)

**Interfaces:**
- Produces: `config::Cli` (clap `Parser`), `config::Config { bind, db, admin_user, admin_password, setup_token, session_key }`, `config::Config::load(cli: Cli) -> Result<Config, figment::Error>`, `config::Config::is_loopback_bind(&self) -> bool`, `config::SetupTokenPolicy { Open, Required(Option<String>) }`, `config::Config::setup_token_policy(&self) -> SetupTokenPolicy`.

- [ ] **Step 1: Add dependencies**

In `src/server/Cargo.toml` `[dependencies]`:
```toml
clap = { version = "4", features = ["derive", "env"] }
figment = { version = "0.10", features = ["toml", "env"] }
```

- [ ] **Step 2: Write failing tests**

Create `src/server/src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_nothing_set() {
        let cfg = Config::default();
        assert_eq!(cfg.bind, "127.0.0.1:30000");
        assert_eq!(cfg.db, "./shadowcat.db");
        assert_eq!(cfg.setup_token, "auto");
        assert!(cfg.admin_user.is_none());
    }

    #[test]
    fn cli_overrides_take_precedence_over_defaults() {
        let cli = Cli {
            bind: Some("0.0.0.0:8080".into()),
            db: None,
            config: Some("/nonexistent/shadowcat.toml".into()),
            admin_user: Some("ops".into()),
            admin_password: None,
            setup_token: None,
            session_key: None,
        };
        let cfg = Config::load(cli).expect("load");
        assert_eq!(cfg.bind, "0.0.0.0:8080");
        assert_eq!(cfg.db, "./shadowcat.db"); // untouched default
        assert_eq!(cfg.admin_user.as_deref(), Some("ops"));
    }

    #[test]
    fn loopback_detection() {
        let mut cfg = Config::default();
        assert!(cfg.is_loopback_bind());
        cfg.bind = "0.0.0.0:30000".into();
        assert!(!cfg.is_loopback_bind());
        cfg.bind = "[::1]:30000".into();
        assert!(cfg.is_loopback_bind());
    }

    #[test]
    fn setup_token_policy_auto_derives_from_bind() {
        let mut cfg = Config::default(); // auto + loopback
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
        cfg.bind = "0.0.0.0:30000".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(None)));
        cfg.setup_token = "off".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Open));
        cfg.setup_token = "required".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(None)));
        cfg.setup_token = "s3cret".into();
        assert!(matches!(cfg.setup_token_policy(), SetupTokenPolicy::Required(Some(ref v)) if v == "s3cret"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p shadowcat config`
Expected: FAIL — `Config` / `Cli` / `SetupTokenPolicy` not defined.

- [ ] **Step 4: Implement**

Prepend to `src/server/src/config.rs`:
```rust
use std::net::{SocketAddr, ToSocketAddrs};

use clap::Parser;
use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

/// CLI flags. Every field is optional so it only overrides lower layers when
/// explicitly provided.
#[derive(Parser, Debug, Default)]
#[command(name = "shadowcat")]
pub struct Cli {
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub db: Option<String>,
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub admin_user: Option<String>,
    #[arg(long)]
    pub admin_password: Option<String>,
    #[arg(long)]
    pub setup_token: Option<String>,
    #[arg(long)]
    pub session_key: Option<String>,
}

/// Effective server configuration after layering. Precedence (high→low):
/// CLI flag > SHADOWCAT_* env > TOML file > built-in default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bind: String,
    pub db: String,
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
    /// "auto" | "off" | "required" | <explicit token>.
    pub setup_token: String,
    pub session_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:30000".into(),
            db: "./shadowcat.db".into(),
            admin_user: None,
            admin_password: None,
            setup_token: "auto".into(),
            session_key: None,
        }
    }
}

/// Resolved setup-window policy. `Required(None)` means a token is required but
/// none was supplied — the server generates one at boot.
#[derive(Debug, Clone)]
pub enum SetupTokenPolicy {
    Open,
    Required(Option<String>),
}

impl Config {
    /// Layer file + env over defaults via figment, then apply CLI overrides in
    /// code so CLI strictly wins (figment cannot easily skip `None` CLI fields).
    pub fn load(cli: Cli) -> Result<Self, figment::Error> {
        let config_path = cli.config.clone().unwrap_or_else(|| "shadowcat.toml".into());
        let mut cfg: Config = Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(&config_path)) // missing file is ignored
            .merge(Env::prefixed("SHADOWCAT_"))
            .extract()?;

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
        Ok(cfg)
    }

    /// True when the bind host resolves to a loopback address. `0.0.0.0` /
    /// non-loopback hosts are treated as exposed. On parse failure, default to
    /// the safe answer (not loopback) so the token is required.
    pub fn is_loopback_bind(&self) -> bool {
        self.bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr: SocketAddr| addr.ip().is_loopback())
            .unwrap_or(false)
    }

    pub fn setup_token_policy(&self) -> SetupTokenPolicy {
        match self.setup_token.as_str() {
            "off" => SetupTokenPolicy::Open,
            "required" => SetupTokenPolicy::Required(None),
            "auto" => {
                if self.is_loopback_bind() {
                    SetupTokenPolicy::Open
                } else {
                    SetupTokenPolicy::Required(None)
                }
            }
            explicit => SetupTokenPolicy::Required(Some(explicit.to_string())),
        }
    }
}
```

Add to `src/server/src/lib.rs`:
```rust
pub mod config;
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat config`
Expected: PASS (4 tests). Note: `to_socket_addrs` on `"[::1]:30000"` resolves without network I/O.

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/config.rs src/server/src/lib.rs
git commit -m "feat(m3): add layered CLI/env/TOML configuration"
```

---

### Task 4: HTTP scaffold — `AppError`, `AppState`, `/health`, tracing + request-id, boot

**Files:**
- Create: `src/server/src/http/mod.rs`, `src/server/src/http/error.rs`, `src/server/src/http/routes.rs`
- Modify: `src/server/src/lib.rs` (add `pub mod http;`)
- Modify: `src/server/src/main.rs` (real entrypoint)
- Modify: `src/server/Cargo.toml` (axum, tower, tower-http, tracing, tracing-subscriber, anyhow; dev: axum-test, serde_json already present)

**Interfaces:**
- Produces: `http::AppState { repo: Arc<SqliteRepository>, config: Arc<Config>, setup_token: Option<String>, initialized: Arc<AtomicBool> }`; `http::router(state: AppState) -> axum::Router`; `http::error::AppError { Unauthorized, Forbidden, Conflict(String), BadRequest(String), Internal }` implementing `IntoResponse`.
- Consumes: `data::SqliteRepository` (Task 1), `config::Config` (Task 3).

- [ ] **Step 1: Add dependencies**

In `src/server/Cargo.toml`:
```toml
# [dependencies]
axum = "0.8"
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "request-id", "util"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"

# [dev-dependencies]
axum-test = "17"
```

- [ ] **Step 2: Write the failing integration test for `/health`**

Create `src/server/src/http/mod.rs`:
```rust
pub mod error;
pub mod routes;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::config::Config;
use crate::data::sqlite::SqliteRepository;

/// Shared handler state. `initialized` caches "an admin exists" so the init
/// gate (Task 8) avoids a DB hit per request; `setup_token`, when `Some`, is the
/// value `/api/setup` requires.
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepository>,
    pub config: Arc<Config>,
    pub setup_token: Option<String>,
    pub initialized: Arc<AtomicBool>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(routes::health))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    async fn test_state() -> AppState {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        AppState {
            repo: Arc::new(repo),
            config: Arc::new(Config::default()),
            setup_token: None,
            initialized: Arc::new(AtomicBool::new(false)),
        }
    }

    #[tokio::test]
    async fn health_reports_db_connected() {
        let server = axum_test::TestServer::new(router(test_state().await)).unwrap();
        let res = server.get("/health").await;
        res.assert_status_ok();
        let body: crate::health::HealthStatus = res.json();
        assert_eq!(body.status, "ok");
        assert!(body.db_connected);
    }
}
```

Create `src/server/src/http/error.rs`:
```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Handler error mapped to a clean status code. 5xx detail is logged, never
/// returned in the body.
#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    Forbidden,
    Conflict(String),
    BadRequest(String),
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (status, Json(ErrorBody { error: msg })).into_response()
    }
}
```

Create `src/server/src/http/routes.rs`:
```rust
use axum::extract::State;
use axum::Json;

use crate::health::HealthStatus;
use crate::http::AppState;

/// Liveness + DB connectivity probe.
pub async fn health(State(state): State<AppState>) -> Json<HealthStatus> {
    let connected = sqlx::query("SELECT 1")
        .fetch_one(state.repo.pool())
        .await
        .is_ok();
    Json(HealthStatus::ok(connected))
}
```

Add to `src/server/src/lib.rs`:
```rust
pub mod http;
```

- [ ] **Step 3: Run to verify it fails (then passes once it compiles)**

Run: `cargo test -p shadowcat http::`
Expected: first FAIL/compile-error until all three files exist, then PASS (1 test). If `axum-test` major differs, pin the version that matches axum 0.8 and note it in the commit.

- [ ] **Step 4: Wire the real `main.rs` with tracing + request-id middleware**

Replace `src/server/src/main.rs` entirely:
```rust
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use shadowcat::config::{Cli, Config};
use shadowcat::data::sqlite::SqliteRepository;
use shadowcat::http::{self, AppState};

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli)?;
    init_tracing();

    let repo = SqliteRepository::connect(&config.db).await?;
    let initialized = repo.admin_exists().await?;

    let state = AppState {
        repo: Arc::new(repo),
        config: Arc::new(config.clone()),
        setup_token: None, // resolved in Task 8
        initialized: Arc::new(AtomicBool::new(initialized)),
    };

    let app = http::router(state);
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(bind = %config.bind, "shadowcat listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Structured logging filtered by SHADOWCAT_LOG (falling back to RUST_LOG, then "info").
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = std::env::var("SHADOWCAT_LOG")
        .ok()
        .and_then(|s| EnvFilter::try_new(s).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
```

Add the request-id + trace layers to `http::router` in `src/server/src/http/mod.rs`. Replace the `router` body:
```rust
pub fn router(state: AppState) -> Router {
    use tower::ServiceBuilder;
    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::trace::TraceLayer;

    Router::new()
        .route("/health", get(routes::health))
        .layer(
            // Outermost→innermost: stamp a request id, trace the span, then
            // propagate the id onto the response.
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .with_state(state)
}
```

- [ ] **Step 5: Verify build + tests + a manual boot**

Run: `cargo test -p shadowcat`
Expected: PASS (all suites).
Run: `cargo build -p shadowcat`
Expected: builds a binary. (Manual smoke — optional: `cargo run -p shadowcat -- --db sqlite::memory:` logs `shadowcat listening` then Ctrl-C.)

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/http/ src/server/src/lib.rs src/server/src/main.rs
git commit -m "feat(m3): axum scaffold with /health, tracing, and request ids"
```

---

### Task 5: Embedded static assets + transitional auth pages

**Files:**
- Create: `src/server/static/index.html`, `setup.html`, `login.html`, `auth.js`, `styles.css`
- Create: `src/server/src/http/embed.rs`
- Modify: `src/server/src/http/mod.rs` (add `pub mod embed;`, register fallback)
- Modify: `src/server/Cargo.toml` (rust-embed, mime_guess)

**Interfaces:**
- Produces: `http::embed::static_handler(uri: axum::http::Uri) -> axum::response::Response`, serving files from `src/server/static/` by path (root `/` → `index.html`), 404 on miss.

- [ ] **Step 1: Add dependencies**

In `src/server/Cargo.toml`:
```toml
rust-embed = "8"
mime_guess = "2"
```

- [ ] **Step 2: Author the static files**

`src/server/static/index.html`:
```html
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>shadowcat</title><link rel="stylesheet" href="/styles.css"></head>
<body><main><h1>shadowcat</h1><p>Server is running. The full client UI is not yet built.</p></main></body>
</html>
```

`src/server/static/setup.html`:
```html
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>shadowcat — first-run setup</title><link rel="stylesheet" href="/styles.css"></head>
<body><main>
  <h1>Create the admin account</h1>
  <form id="setup-form">
    <label>Username <input name="username" autocomplete="username" required></label>
    <label>Password <input name="password" type="password" autocomplete="new-password" required></label>
    <label>Setup token <input name="token" autocomplete="off" placeholder="only if required"></label>
    <button type="submit">Create admin</button>
  </form>
  <p id="msg" role="status"></p>
</main><script src="/auth.js"></script></body>
</html>
```

`src/server/static/login.html`:
```html
<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>shadowcat — log in</title><link rel="stylesheet" href="/styles.css"></head>
<body><main>
  <h1>Log in</h1>
  <form id="login-form">
    <label>Username <input name="username" autocomplete="username" required></label>
    <label>Password <input name="password" type="password" autocomplete="current-password" required></label>
    <button type="submit">Log in</button>
  </form>
  <p id="msg" role="status"></p>
</main><script src="/auth.js"></script></body>
</html>
```

`src/server/static/auth.js`:
```javascript
// Transitional vanilla-JS driver for the M3 setup/login forms. Replaced by the
// Svelte auth UI later. Posts JSON to the auth API and reports status inline.
async function post(url, payload) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  return res;
}

function fields(form) {
  return Object.fromEntries(new FormData(form).entries());
}

const setupForm = document.getElementById("setup-form");
if (setupForm) {
  setupForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const f = fields(setupForm);
    const body = { username: f.username, password: f.password };
    if (f.token) body.token = f.token;
    const res = await post("/api/setup", body);
    document.getElementById("msg").textContent = res.ok
      ? "Admin created. You can now log in."
      : `Setup failed (${res.status}).`;
    if (res.ok) window.location.href = "/login.html";
  });
}

const loginForm = document.getElementById("login-form");
if (loginForm) {
  loginForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const res = await post("/api/login", fields(loginForm));
    document.getElementById("msg").textContent = res.ok
      ? "Logged in."
      : "Invalid username or password.";
    if (res.ok) window.location.href = "/";
  });
}
```

`src/server/static/styles.css`:
```css
:root { color-scheme: dark; }
body { font-family: system-ui, sans-serif; margin: 0; background: #1e1e2e; color: #cdd6f4; }
main { max-width: 28rem; margin: 4rem auto; padding: 0 1.5rem; }
h1 { font-size: 1.4rem; }
form { display: grid; gap: 0.75rem; }
label { display: grid; gap: 0.25rem; }
input { padding: 0.5rem; border-radius: 0.375rem; border: 1px solid #45475a; background: #313244; color: inherit; }
button { padding: 0.5rem; border-radius: 0.375rem; border: 0; background: #89b4fa; color: #1e1e2e; font-weight: 600; cursor: pointer; }
</style>
```
(Note: the trailing `</style>` line in `styles.css` is a typo — the file is plain CSS with no tag. Omit it.)

- [ ] **Step 3: Write the failing test**

Create `src/server/src/http/embed.rs`:
```rust
#[cfg(test)]
mod tests {
    use crate::http::tests::test_state;
    use crate::http::router;

    #[tokio::test]
    async fn serves_index_at_root_and_named_assets() {
        let server = axum_test::TestServer::new(router(test_state().await)).unwrap();

        let root = server.get("/").await;
        root.assert_status_ok();
        assert!(root.text().contains("Server is running"));

        let setup = server.get("/setup.html").await;
        setup.assert_status_ok();
        assert!(setup.text().contains("Create the admin account"));

        let missing = server.get("/does-not-exist").await;
        missing.assert_status_not_found();
    }
}
```

To reuse `test_state`, change `src/server/src/http/mod.rs` test module to expose it: make `mod tests` into `pub(crate) mod tests` and `async fn test_state` into `pub(crate) async fn test_state`.

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p shadowcat http::embed`
Expected: FAIL — `static_handler` not registered; `/` and `/setup.html` 404.

- [ ] **Step 5: Implement the embed handler and register the fallback**

Prepend to `src/server/src/http/embed.rs`:
```rust
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Embedded transitional auth bundle. Embeds `src/server/static/` into the
/// binary. SEAM: when the Vite client bundle exists, repoint `folder` at the
/// client `dist/` output — callers of `static_handler` do not change.
#[derive(rust_embed::RustEmbed)]
#[folder = "static/"]
struct StaticAssets;

/// Serve an embedded asset by request path; `/` maps to `index.html`.
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match StaticAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
```

In `src/server/src/http/mod.rs`: add `pub mod embed;` near the top, and add a fallback to the router (before `.layer(...)`):
```rust
        .route("/health", get(routes::health))
        .fallback(embed::static_handler)
```

- [ ] **Step 6: Run to verify it passes**

Run: `cargo test -p shadowcat http::`
Expected: PASS (health + embed tests). `#[folder = "static/"]` is resolved relative to `src/server/` (the crate manifest dir).

- [ ] **Step 7: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/static/ src/server/src/http/
git commit -m "feat(m3): embed transitional static auth pages via rust-embed"
```

---

### Task 6: Session layer, session-key persistence, and auth extractors

**Files:**
- Create: `src/server/src/auth/session.rs`
- Modify: `src/server/src/auth/mod.rs` (add `pub mod session;`)
- Modify: `src/server/Cargo.toml` (tower-sessions, tower-sessions-sqlx-store, base64 — versions per the compatibility check above)

**Interfaces:**
- Produces:
  - `auth::session::SessionUser { id: Uuid, username: String, role: ServerRole }` (Serialize/Deserialize) — the value stored under session key `"user"`.
  - `auth::session::AuthUser { id: Uuid, username: String, role: ServerRole }` — `FromRequestParts<AppState>` extractor; 401 when no session user.
  - `auth::session::AdminUser(pub AuthUser)` — `FromRequestParts<AppState>`; 403 when role ≠ Admin.
  - `auth::session::session_layer(repo: &SqliteRepository, config: &Config) -> anyhow::Result<SessionManagerLayer<...>>` — builds the store (runs its migration), loads/creates the signing key, sets `Secure`/`SameSite=Lax`.
- Consumes: `http::error::AppError` (Task 4), `auth::role::ServerRole` (Task 1), `config::Config` (Task 3).

- [ ] **Step 1: Add dependencies (use versions from the compatibility check)**

In `src/server/Cargo.toml` (example pin — confirm against the check):
```toml
tower-sessions = "0.13"
tower-sessions-sqlx-store = { version = "0.14", features = ["sqlite"] }
base64 = "0.22"
```

- [ ] **Step 2: Write the failing test (extractors via a throwaway test router)**

Create `src/server/src/auth/session.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::role::ServerRole;
    use crate::http::AppState;
    use axum::routing::{get, post};
    use axum::Router;
    use tower_sessions::Session;
    use uuid::Uuid;

    // Test-only routes that exercise the extractors without the prod surface.
    async fn login_as_admin(session: Session) -> &'static str {
        session
            .insert(
                "user",
                SessionUser { id: Uuid::from_u128(1), username: "a".into(), role: ServerRole::Admin },
            )
            .await
            .unwrap();
        "ok"
    }
    async fn login_as_user(session: Session) -> &'static str {
        session
            .insert(
                "user",
                SessionUser { id: Uuid::from_u128(2), username: "u".into(), role: ServerRole::User },
            )
            .await
            .unwrap();
        "ok"
    }
    async fn whoami(user: AuthUser) -> String {
        user.username
    }
    async fn admin_only(_admin: AdminUser) -> &'static str {
        "admin"
    }

    async fn harness() -> (axum_test::TestServer, ()) {
        let state = crate::http::tests::test_state().await;
        let layer = session_layer(&state.repo, &state.config).await.unwrap();
        let app = Router::new()
            .route("/t/login-admin", post(login_as_admin))
            .route("/t/login-user", post(login_as_user))
            .route("/t/me", get(whoami))
            .route("/t/admin", get(admin_only))
            .layer(layer)
            .with_state(state);
        (axum_test::TestServer::builder().save_cookies().build(app).unwrap(), ())
    }

    #[tokio::test]
    async fn auth_user_requires_session() {
        let (server, _) = harness().await;
        server.get("/t/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_extractor_rejects_non_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-user").await.assert_status_ok();
        server.get("/t/me").await.assert_status_ok(); // any user passes AuthUser
        server.get("/t/admin").await.assert_status(axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_extractor_accepts_admin() {
        let (server, _) = harness().await;
        server.post("/t/login-admin").await.assert_status_ok();
        server.get("/t/admin").await.assert_status_ok();
    }

    #[tokio::test]
    async fn session_key_is_stable_across_loads() {
        let repo = crate::data::sqlite::SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let cfg = crate::config::Config::default();
        let k1 = load_or_create_key(&repo, &cfg).await.unwrap();
        let k2 = load_or_create_key(&repo, &cfg).await.unwrap();
        assert_eq!(k1.master(), k2.master(), "persisted key must be reused");
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p shadowcat auth::session`
Expected: FAIL — `SessionUser`/`AuthUser`/`AdminUser`/`session_layer`/`load_or_create_key` not defined.

- [ ] **Step 4: Implement**

Prepend to `src/server/src/auth/session.rs`:
```rust
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tower_sessions::cookie::time::Duration;
use tower_sessions::cookie::{Key, SameSite};
use tower_sessions::{Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;
use uuid::Uuid;

use crate::auth::role::ServerRole;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;
use crate::http::AppState;

const SESSION_USER_KEY: &str = "user";
const SESSION_KEY_SETTING: &str = "session_key";

/// Identity persisted in the session store after login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUser {
    pub id: Uuid,
    pub username: String,
    pub role: ServerRole,
}

/// Any authenticated user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub username: String,
    pub role: ServerRole,
}

/// An authenticated user whose server role is Admin.
pub struct AdminUser(pub AuthUser);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Unauthorized)?;
        let user: Option<SessionUser> = session
            .get(SESSION_USER_KEY)
            .await
            .map_err(|_| AppError::Internal)?;
        let u = user.ok_or(AppError::Unauthorized)?;
        Ok(AuthUser { id: u.id, username: u.username, role: u.role })
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role == ServerRole::Admin {
            Ok(AdminUser(user))
        } else {
            Err(AppError::Forbidden)
        }
    }
}

/// Load the persisted session signing key, or generate + persist one. An
/// explicit `config.session_key` (base64) overrides storage.
pub async fn load_or_create_key(repo: &SqliteRepository, config: &Config) -> anyhow::Result<Key> {
    if let Some(explicit) = &config.session_key {
        let raw = base64::engine::general_purpose::STANDARD.decode(explicit)?;
        return Ok(Key::from(&raw));
    }
    if let Some(stored) = repo.get_setting(SESSION_KEY_SETTING).await? {
        let raw = base64::engine::general_purpose::STANDARD.decode(stored)?;
        return Ok(Key::from(&raw));
    }
    let key = Key::generate();
    let encoded = base64::engine::general_purpose::STANDARD.encode(key.master());
    repo.set_setting(SESSION_KEY_SETTING, &encoded).await?;
    Ok(key)
}

/// Build the signed, DB-backed session layer. Cookie is `Secure` only on a
/// non-loopback bind (so loopback dev over http still works).
pub async fn session_layer(
    repo: &SqliteRepository,
    config: &Config,
) -> anyhow::Result<SessionManagerLayer<SqliteStore>> {
    let store = SqliteStore::new(repo.pool().clone());
    store.migrate().await?;
    let key = load_or_create_key(repo, config).await?;
    Ok(SessionManagerLayer::new(store)
        .with_secure(!config.is_loopback_bind())
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::days(7)))
        .with_signed(key))
}
```

Add `pub mod session;` to `src/server/src/auth/mod.rs`.

> API note: `SqliteStore::new` takes a `sqlx::SqlitePool` by value; `repo.pool()` returns `&SqlitePool`, so `.clone()` (cheap Arc clone). If the pinned `tower-sessions` exposes signing via `tower_sessions::cookie::Key` differently (e.g. a `PrivateCookie`/`SignedCookie` wrapper), adapt `with_signed`; the rest is stable. Confirm `Key::from(&Vec<u8>)` vs `Key::from(&[u8])` against the pinned cookie crate.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat auth::session`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add src/server/Cargo.toml src/server/Cargo.lock src/server/src/auth/
git commit -m "feat(m3): DB-backed signed sessions and auth extractors"
```

---

### Task 7: Login / logout / me endpoints

**Files:**
- Modify: `src/server/src/http/routes.rs` (add `login`, `logout`, `me`)
- Modify: `src/server/src/http/mod.rs` (register `/api/login`, `/api/logout`, `/api/me`; attach session layer in `router` for tests)

**Interfaces:**
- Produces handlers: `routes::login`, `routes::logout`, `routes::me`. Request/response types: `LoginRequest { username, password }`, `MeResponse { id, username, server_role }`.
- Consumes: `auth::password::verify_password` (Task 2), `auth::session::{SessionUser, AuthUser, session_layer}` (Task 6), `data::SqliteRepository::user_by_username` (Task 1).

> Router note: from this task on, `router(state)` must attach the session layer so cookies work in tests. Since building the layer is async, add `pub async fn router_with_sessions(state) -> Router` used by tests and `main`; keep the sync `router` for the no-session `/health`+embed tests, or convert all tests to the async builder. This plan converts `router` to async.

- [ ] **Step 1: Convert `router` to async and attach sessions**

In `src/server/src/http/mod.rs`, replace `pub fn router` with:
```rust
pub async fn router(state: AppState) -> Router {
    use tower::ServiceBuilder;
    use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
    use tower_http::trace::TraceLayer;

    let sessions = crate::auth::session::session_layer(&state.repo, &state.config)
        .await
        .expect("session layer");

    Router::new()
        .route("/health", get(routes::health))
        .route("/api/me", get(routes::me))
        .route("/api/login", axum::routing::post(routes::login))
        .route("/api/logout", axum::routing::post(routes::logout))
        .fallback(embed::static_handler)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .layer(sessions)
        .with_state(state)
}
```
Update every `router(...)` test call site (in `http::mod`, `http::embed`, and `auth::session` if it used it) to `router(...).await`. Update `main.rs`: `let app = http::router(state).await;`.

- [ ] **Step 2: Write failing endpoint tests**

Add to the `tests` module in `src/server/src/http/mod.rs`:
```rust
use crate::auth::password::hash_password;
use crate::auth::role::ServerRole;

async fn server_with_user(username: &str, password: &str, role: ServerRole) -> axum_test::TestServer {
    let state = test_state().await;
    let hash = hash_password(password).unwrap();
    state.repo.create_user(username, Some(&hash), role, 0).await.unwrap();
    axum_test::TestServer::builder()
        .save_cookies()
        .build(router(state).await)
        .unwrap()
}

#[tokio::test]
async fn login_success_then_me_then_logout() {
    let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

    server.get("/api/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    let login = server.post("/api/login").json(&serde_json::json!({
        "username": "gm-1", "password": "pw-correct"
    })).await;
    login.assert_status(axum::http::StatusCode::NO_CONTENT);

    let me = server.get("/api/me").await;
    me.assert_status_ok();
    assert!(me.text().contains("gm-1"));

    server.post("/api/logout").await.assert_status(axum::http::StatusCode::NO_CONTENT);
    server.get("/api/me").await.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_rejects_wrong_password_and_unknown_user_identically() {
    let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

    let bad_pw = server.post("/api/login").json(&serde_json::json!({
        "username": "gm-1", "password": "pw-wrong"
    })).await;
    let unknown = server.post("/api/login").json(&serde_json::json!({
        "username": "ghost", "password": "whatever"
    })).await;

    bad_pw.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    unknown.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(bad_pw.text(), unknown.text(), "no user enumeration via body");
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p shadowcat http::tests::login`
Expected: FAIL — `me`/`login`/`logout` handlers not defined.

- [ ] **Step 4: Implement the handlers**

Append to `src/server/src/http/routes.rs`:
```rust
use serde::{Deserialize, Serialize};
use tower_sessions::Session;

use crate::auth::password::verify_password;
use crate::auth::role::ServerRole;
use crate::auth::session::{AuthUser, SessionUser};
use crate::http::error::AppError;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub id: uuid::Uuid,
    pub username: String,
    pub server_role: ServerRole,
}

/// Current session identity, or 401.
pub async fn me(user: AuthUser) -> Json<MeResponse> {
    Json(MeResponse { id: user.id, username: user.username, server_role: user.role })
}

/// Verify credentials and establish a session. Uniform 401 on unknown user or
/// wrong password — no enumeration. Always runs a verify to keep timing flat.
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Json(body): Json<LoginRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    // A valid-shaped Argon2id PHC for a throwaway password; verified against
    // when the user is unknown so both paths do equal work.
    const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzYWx0$3s8m1 Z9Qe2J5m8b0u2Yx3oQ1k7c5n9w0a2b4c6d8e0";

    let record = state
        .repo
        .user_by_username(&body.username)
        .await
        .map_err(|_| AppError::Internal)?;

    let ok = match &record {
        Some(u) => u
            .password_hash
            .as_deref()
            .map(|h| verify_password(&body.password, h))
            .unwrap_or(false),
        None => {
            let _ = verify_password(&body.password, DUMMY_PHC);
            false
        }
    };
    if !ok {
        return Err(AppError::Unauthorized);
    }
    let u = record.expect("ok implies record present");
    session
        .insert("user", SessionUser { id: u.id, username: u.username, role: u.server_role })
        .await
        .map_err(|_| AppError::Internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Destroy the session.
pub async fn logout(session: Session) -> axum::http::StatusCode {
    let _ = session.flush().await;
    axum::http::StatusCode::NO_CONTENT
}
```
> Note: `DUMMY_PHC` above must be a real, parseable Argon2id PHC string. Generate one once with `hash_password("x")` in a scratch test and paste the exact output (the placeholder here will fail to parse, which still yields `false` but does no hashing work — to get the timing-flattening benefit, paste a valid PHC). The security property that matters (uniform response) holds regardless; the constant-time aspect is best-effort.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p shadowcat http::`
Expected: PASS (health, embed, both login tests).

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/
git commit -m "feat(m3): login/logout/me endpoints with non-enumerating auth"
```

---

### Task 8: First-run setup, setup token, init-gate, headless bootstrap

**Files:**
- Create: `src/server/src/auth/setup.rs`
- Create: `src/server/src/http/middleware.rs`
- Modify: `src/server/src/auth/mod.rs` (add `pub mod setup;`)
- Modify: `src/server/src/http/routes.rs` (add `setup` handler)
- Modify: `src/server/src/http/mod.rs` (register `/api/setup`; apply init-gate; resolve `setup_token` in a state builder)
- Modify: `src/server/src/main.rs` (resolve setup token, run bootstrap, set `initialized`)

**Interfaces:**
- Produces: `auth::setup::create_admin(repo, username, password, now) -> Result<Uuid, AppError>`; `auth::setup::bootstrap_admin(repo, config) -> anyhow::Result<bool>` (true if it seeded); `auth::setup::now_millis() -> i64`; `http::middleware::init_gate` (axum middleware fn); `http::AppState::resolve_setup_token(config) -> Option<String>`.
- Consumes: `config::SetupTokenPolicy` (Task 3), `auth::password::hash_password` (Task 2), `data::SqliteRepository::admin_exists`/`create_user` (Task 1).

- [ ] **Step 1: Implement `create_admin` / `bootstrap_admin` with failing tests**

Create `src/server/src/auth/setup.rs`:
```rust
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::auth::password::hash_password;
use crate::auth::role::ServerRole;
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;
use crate::http::error::AppError;

/// Wall-clock milliseconds since the epoch. Used for `users.created_at`.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Single audited path that hashes a password and writes an admin user.
pub async fn create_admin(
    repo: &SqliteRepository,
    username: &str,
    password: &str,
    now: i64,
) -> Result<Uuid, AppError> {
    let hash = hash_password(password).map_err(|_| AppError::Internal)?;
    repo.create_user(username, Some(&hash), ServerRole::Admin, now)
        .await
        .map_err(|_| AppError::Internal)
}

/// Seed the admin from config when one is configured and none exists. Returns
/// whether it created an account. The remote-hosting path.
pub async fn bootstrap_admin(repo: &SqliteRepository, config: &Config) -> anyhow::Result<bool> {
    if let (Some(u), Some(p)) = (&config.admin_user, &config.admin_password) {
        if !repo.admin_exists().await? {
            create_admin(repo, u, p, now_millis())
                .await
                .map_err(|_| anyhow::anyhow!("bootstrap admin creation failed"))?;
            tracing::info!(username = %u, "bootstrapped admin from config");
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_seeds_admin_once_then_is_idempotent() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let mut cfg = Config::default();
        cfg.admin_user = Some("ops".into());
        cfg.admin_password = Some("pw-bootstrap".into());

        assert!(bootstrap_admin(&repo, &cfg).await.unwrap());
        assert!(repo.admin_exists().await.unwrap());
        // Second call: admin already exists → no-op.
        assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
    }

    #[tokio::test]
    async fn bootstrap_noop_without_config_creds() {
        let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
        let cfg = Config::default();
        assert!(!bootstrap_admin(&repo, &cfg).await.unwrap());
        assert!(!repo.admin_exists().await.unwrap());
    }
}
```
Add `pub mod setup;` to `src/server/src/auth/mod.rs`.

- [ ] **Step 2: Run the setup unit tests**

Run: `cargo test -p shadowcat auth::setup`
Expected: PASS (2 tests).

- [ ] **Step 3: Add the setup-token resolver + init-gate with failing tests**

Add to `src/server/src/http/mod.rs` (impl block + a state builder):
```rust
impl AppState {
    /// Resolve the token `/api/setup` will require. `None` = open window.
    pub fn resolve_setup_token(config: &Config) -> Option<String> {
        use crate::config::SetupTokenPolicy;
        match config.setup_token_policy() {
            SetupTokenPolicy::Open => None,
            SetupTokenPolicy::Required(Some(v)) => Some(v),
            SetupTokenPolicy::Required(None) => {
                let token = uuid::Uuid::new_v4().simple().to_string();
                tracing::info!(%token, "setup token required; provide it on /setup.html");
                Some(token)
            }
        }
    }
}
```

Create `src/server/src/http/middleware.rs`:
```rust
use std::sync::atomic::Ordering;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use crate::http::AppState;

/// While uninitialized, funnel everything except the setup API, the setup page,
/// and static assets to `/setup.html`. Once an admin exists (cached flag), pass
/// through. Coupling: `/api/setup` flips `initialized` after creating the admin.
pub async fn init_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.initialized.load(Ordering::Relaxed) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    let allowed = path == "/api/setup"
        || path == "/setup.html"
        || path == "/health"
        || path.ends_with(".js")
        || path.ends_with(".css");
    if allowed {
        next.run(req).await
    } else {
        Redirect::to("/setup.html").into_response()
    }
}
```

Add failing tests to the `tests` module in `src/server/src/http/mod.rs`:
```rust
async fn fresh_server() -> axum_test::TestServer {
    // Uninitialized state, open token window (loopback default).
    let state = test_state().await;
    axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap()
}

#[tokio::test]
async fn setup_creates_admin_then_closes() {
    let server = fresh_server().await;

    // Uninitialized: a normal page redirects to setup.
    let redirect = server.get("/").await;
    redirect.assert_status(axum::http::StatusCode::SEE_OTHER);

    let setup = server.post("/api/setup").json(&serde_json::json!({
        "username": "admin", "password": "pw-admin"
    })).await;
    setup.assert_status(axum::http::StatusCode::NO_CONTENT);

    // Now initialized: second setup is a conflict, and "/" serves index.
    server.post("/api/setup").json(&serde_json::json!({
        "username": "x", "password": "y"
    })).await.assert_status(axum::http::StatusCode::CONFLICT);
    server.get("/").await.assert_status_ok();

    // The created admin can log in.
    server.post("/api/login").json(&serde_json::json!({
        "username": "admin", "password": "pw-admin"
    })).await.assert_status(axum::http::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn setup_requires_token_when_policy_demands_it() {
    let mut state = test_state().await;
    // Force a required token regardless of bind.
    let mut cfg = crate::config::Config::default();
    cfg.setup_token = "the-token".into();
    state.config = std::sync::Arc::new(cfg.clone());
    state.setup_token = AppState::resolve_setup_token(&cfg);
    let server = axum_test::TestServer::builder().save_cookies().build(router(state).await).unwrap();

    server.post("/api/setup").json(&serde_json::json!({
        "username": "admin", "password": "pw"
    })).await.assert_status(axum::http::StatusCode::FORBIDDEN);

    server.post("/api/setup").json(&serde_json::json!({
        "username": "admin", "password": "pw", "token": "the-token"
    })).await.assert_status(axum::http::StatusCode::NO_CONTENT);
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p shadowcat http::tests::setup`
Expected: FAIL — `setup` handler + init-gate not wired.

- [ ] **Step 5: Implement the `setup` handler and wire the gate**

Append to `src/server/src/http/routes.rs`:
```rust
use crate::auth::setup::{create_admin, now_millis};
use std::sync::atomic::Ordering;

#[derive(Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    pub token: Option<String>,
}

/// First-run admin creation. Gated: 409 once initialized; 403 on token mismatch
/// when a token is required. Flips `initialized` so the gate opens.
pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupRequest>,
) -> Result<axum::http::StatusCode, AppError> {
    if state.initialized.load(Ordering::Relaxed)
        || state.repo.admin_exists().await.map_err(|_| AppError::Internal)?
    {
        return Err(AppError::Conflict("server already initialized".into()));
    }
    if let Some(expected) = &state.setup_token {
        if body.token.as_deref() != Some(expected.as_str()) {
            return Err(AppError::Forbidden);
        }
    }
    create_admin(&state.repo, &body.username, &body.password, now_millis()).await?;
    state.initialized.store(true, Ordering::Relaxed);
    Ok(axum::http::StatusCode::NO_CONTENT)
}
```

In `src/server/src/http/mod.rs`: add `pub mod middleware;` near the top, register the route, and apply the gate. Update the router body:
```rust
        .route("/health", get(routes::health))
        .route("/api/me", get(routes::me))
        .route("/api/login", axum::routing::post(routes::login))
        .route("/api/logout", axum::routing::post(routes::logout))
        .route("/api/setup", axum::routing::post(routes::setup))
        .fallback(embed::static_handler)
        .layer(axum::middleware::from_fn_with_state(state.clone(), middleware::init_gate))
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(PropagateRequestIdLayer::x_request_id()),
        )
        .layer(sessions)
        .with_state(state)
```
(The `from_fn_with_state` needs `state` before it is moved into `with_state`; clone it as shown.)

- [ ] **Step 6: Resolve the token + run bootstrap in `main.rs`**

In `src/server/src/main.rs`, after `Config::load` and `SqliteRepository::connect`, before building `AppState`:
```rust
    // Headless bootstrap (remote hosting): seed admin from config if present.
    let seeded = shadowcat::auth::setup::bootstrap_admin(&repo, &config).await?;
    let initialized = seeded || repo.admin_exists().await?;
    let setup_token = AppState::resolve_setup_token(&config);
```
and set `setup_token` + `initialized` fields in the `AppState { .. }` literal (replacing the `setup_token: None` placeholder and the prior `initialized` line).

- [ ] **Step 7: Run the full suite**

Run: `cargo test -p shadowcat`
Expected: PASS (all tasks' tests).
Run: `cargo build -p shadowcat`
Expected: clean build.

- [ ] **Step 8: Commit**

```bash
git add src/server/src/auth/ src/server/src/http/ src/server/src/main.rs
git commit -m "feat(m3): first-run setup flow, setup token, init-gate, headless bootstrap"
```

---

### Task 9: Workspace lint/format gate + docs sync

**Files:**
- Modify: `docs/PLAN.md` (mark M3 complete), `docs/TODO.md` (confirm M5 validation-wiring deferral still accurate)

- [ ] **Step 1: Format + clippy**

Run: `cargo fmt -p shadowcat -- --check` then `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: no diffs, no warnings. Fix any inline.

- [ ] **Step 2: Full test run**

Run: `cargo test -p shadowcat`
Expected: PASS.

- [ ] **Step 3: ts-rs sync check (no TS types changed in M3, but verify CI green path)**

Run the repo's type-gen command (per M1 setup) and confirm `src/types/generated/` is unchanged. If the project's CI script name differs, use it; M3 adds no `#[derive(TS)]` types, so this should be a no-op.

- [ ] **Step 4: Docs sync**

In `docs/PLAN.md`, mark **M3 · Auth + server skeleton** complete (consistent with how M1/M2 are marked). Confirm `docs/TODO.md`'s data-layer deferral note still reads "M3/M5 … HTTP/permission layer" — M3 exposed no document write path, so update it to point solely at **M5** (the validation wiring belongs with document CRUD).

- [ ] **Step 5: Commit**

```bash
git add docs/PLAN.md docs/TODO.md
git commit -m "docs(m3): mark M3 complete and retarget validation-wiring deferral to M5"
```

---

## Self-Review

**Spec coverage:**
- axum boots + runs migrations → Task 4 (boot) + existing `SqliteRepository::connect` migration run; new migration Task 1. ✓
- argon2 → Task 2. ✓
- tower-sessions DB-backed → Task 6. ✓
- server/GM/player/spectator roles → `ServerRole` Task 1; `WorldRole` pre-exists. ✓
- structured logging + request ids + /health → Task 4. ✓
- single-binary rust-embed stub → Task 5. ✓
- first-run setup + token (bind-derived, overridable) + headless bootstrap → Tasks 3 (policy) + 8. ✓
- layered config → Task 3. ✓
- static HTML auth pages → Task 5. ✓
- session key persisted in `settings` → Task 6. ✓
- M5 validation-wiring stays deferred → Task 9 doc note; not implemented. ✓
- AuthUser + AdminUser extractors → Task 6 (AdminUser exercised via test-only router, since M3 has no admin-only endpoint — kept per spec, not trimmed). ✓

**Placeholder scan:** One deliberate, flagged item — `DUMMY_PHC` in Task 7 must be replaced with a real PHC string generated via `hash_password`; the step calls this out explicitly with how to produce it. The `setup_token: None` in Task 4's `main.rs`/AppState is a forward placeholder resolved in Task 8 Step 6 (noted in both places). No silent TBDs.

**Type consistency:** `ServerRole`, `UserRecord`, `SessionUser`, `AuthUser`, `AppState` field names (`repo`/`config`/`setup_token`/`initialized`), `AppError` variants, and handler names (`health`/`me`/`login`/`logout`/`setup`) are used consistently across tasks. `router` is async from Task 7 onward — all call sites updated in that task's Step 1.

**Version risk:** tower-sessions / tower-sessions-sqlx-store / sqlx 0.9 pairing, axum-test major, and `cookie::Key` construction are flagged for verification at their tasks; these are pins, not design changes.

## Buddy-check directives

This plan implements security-sensitive auth (argon2 credential handling, signed DB-backed sessions, the open setup window + token, headless credential bootstrap, and non-enumerating login). That qualifies as high-risk. **Outcome of the checkpoint:** the buddy-check was **offered at the execution handoff and ACCEPTED by the user** (2026-06-18). Run `buddy-checking` over the credential/session/setup-token surface — Tasks 2, 6, 7, 8 — after implementation completes, before merge, in addition to the mainline-plan-execution final branch review.
