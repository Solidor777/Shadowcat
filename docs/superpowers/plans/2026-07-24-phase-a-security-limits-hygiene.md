# Phase A — Security, Limits & Server Hygiene Implementation Plan

> **For agentic workers:** On a Fable-class model this plan is executed via the
> `mainline-plan-execution` skill (per user CLAUDE.md). On any other model use
> superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every Phase-A item of the Phase-1 close-out campaign spec
(`docs/superpowers/specs/2026-07-24-phase1-closeout-campaign-design.md`): auth throttling,
invite GC, token-coordinate ingress validation, the six fail-open cell-size defaults, the
`ScenePing` guard, the chat world-scope pin, backup/restore atomicity + quiesce, the
`apply_command` engine gate, and the dice construction guards + test/display fixes.

**Architecture:** All server-side (zero client changes). Each fix lands at the structural
chokepoint its subsystem already defines — the engine-normalization seam, the one shared
`MAX_GATE_WALK_COORD` symbol, `validate_pre_roll`, the session sweep — never a parallel path.

**Tech Stack:** Rust (axum 0.8, sqlx 0.9, tokio 1.52). **No new dependencies.**

## Global Constraints

- No new crate dependencies (`Cargo.toml` `[dependencies]` unchanged).
- Cross-platform: `std::path` only; every touched path op must be valid on the three-OS CI matrix.
- Run cargo via `--manifest-path src/server/Cargo.toml` from the repo root (never `cd src/server` — cwd drift breaks the Edit hook and git paths).
- `dist/` already exists (required by rust-embed at compile time); this plan never rebuilds the client.
- Per task: `cargo test --manifest-path src/server/Cargo.toml` green, then commit. Before the final task: `cargo fmt --manifest-path src/server/Cargo.toml` + `cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings`.
- Never-fork rule: any new bound/predicate shared between two paths must be ONE symbol with an anti-drift test that exercises both paths (template: `ws/room.rs:2653` `publish_move_gate_admissibility_bound_equals_gate_walks`).
- Comment style: present-tense invariants/constraints only; no history, no process meta (project CLAUDE.md).

## Model/Effort directives

Fable-class session: plan written and executed mainline (the `sdd-*` dispatch ladder applies
only to non-Fable sessions). No model/effort switch — Fable 5 at high effort exceeds the
opus/high design tier.

## Buddy-check directives

Phase A is security-sensitive (spec: "Security-sensitive phases (A, B, C, G) get the
two-reviewer pair"). Directives:
- Final whole-branch review: dispatch `shadowcat-spec-reviewer` + `shadowcat-code-reviewer`
  as a two-reviewer pair (not the single mainline review).
- Tasks 2 (auth throttle wiring), 7 (`ScenePing` guard), and 9 (`apply_command` gate) are
  pre-authorized for per-task buddy-checks if the executor judges the diff warrants it.

## Design decisions locked at plan time

1. **Throttle shape:** hand-rolled sliding-window limiter (mirrors `assets.rs::UploadRateLimiter`'s
   proven shape) with string keys; per-identity + per-IP keys; map capacity bound with
   fail-closed overflow. No tower middleware, no new crate.
2. **Client IP:** the server currently serves without connect-info (`main.rs:62-64`), so IP is
   plumbed via `into_make_service_with_connect_info::<SocketAddr>()` + an infallible `ClientIp`
   extractor reading `parts.extensions`. Under the axum-test mock transport IP is `None` and only
   identity keys throttle — tests remain valid.
3. **Throttle is not an oracle:** identity keys count attempts against unknown identities exactly
   like known ones; 429 body is uniform; a throttled request spends zero Argon2 (asserted via
   `verify_count()`).
4. **Quiesce = in-server backup.** CLI backup/restore runs with the server process not running
   (`main.rs` early-returns before pool creation), so an in-process barrier cannot protect a
   cross-process CLI backup against a live server. The quiesce therefore ships as an
   **admin-gated `POST /api/admin/backup`** route (Phase-4 "backup scheduling" needs in-server
   backup anyway) holding a server-wide write barrier that asset writes take in read mode. The
   barrier gates only the asset commit+rename critical section — `VACUUM INTO` is already
   consistent against live DB writers.
5. **`ReplaceDie` invalid-on-`Faces` is silently skipped**, matching `RecalcOp`'s documented
   "invalid ids are silently ignored" semantics — no signature change to `recalculate`.
   Out-of-domain naturals on `Numeric` dice remain allowed (pinned by the existing
   `recalc.rs:459` test as intended GM-override behavior).
6. **Tier-ladder guard lives in `validate_pre_roll`** (the wire boundary where
   `DieKind::validate()` already runs), with a new `RollError::DuplicateTierOffset` variant.
7. **`labeled_consts` sign threads through `Neg` and `Sub`-rhs** (additive contexts); `Mul`/`Div`
   keep the literal value, documented.
8. **`apply_command` mirrors exactly the `/engine` gate** (`validate_engine_tree` + normalized
   `FieldChange` re-derivation): capability/schema/size gates stay absent by design (trusted
   substrate, zero production callers — documented).

---

### Task 1: `AuthThrottle` — keyed sliding-window limiter

**Files:**
- Create: `src/server/src/http/throttle.rs`
- Modify: `src/server/src/http/mod.rs` (declare `pub mod throttle;`, add `AppState` field, update `test_state`)
- Modify: `src/server/src/main.rs:50-57` (AppState construction)
- Modify: `src/server/src/bin/test_server.rs:156-163` (AppState construction)

**Interfaces:**
- Produces: `http::throttle::AuthThrottle` with `pub fn new() -> Self` and
  `pub fn check(&self, key: &str, now_ms: i64, per_min: usize) -> bool`;
  constants `LOGIN_PER_MIN_PER_IDENTITY: usize = 10`, `LOGIN_PER_MIN_PER_IP: usize = 30`,
  `INVITE_PER_MIN_PER_ACCOUNT: usize = 10`, `INVITE_PER_MIN_PER_IP: usize = 30`,
  `MAX_TRACKED_KEYS: usize = 65_536`; `AppState.auth_throttle: Arc<AuthThrottle>`.
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the failing tests** (in `throttle.rs`'s own `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trips_after_budget_then_window_slides() {
        let t = AuthThrottle::new();
        assert!(t.check("login:u:alice", 1_000, 2));
        assert!(t.check("login:u:alice", 1_500, 2));
        assert!(!t.check("login:u:alice", 1_800, 2)); // 3rd within 60s window
        assert!(t.check("login:u:alice", 62_001, 2)); // window slid
    }

    #[test]
    fn keys_are_independent() {
        let t = AuthThrottle::new();
        assert!(t.check("login:u:alice", 1_000, 1));
        assert!(t.check("login:u:bob", 1_000, 1));
        assert!(!t.check("login:u:alice", 1_100, 1));
    }

    #[test]
    fn at_capacity_new_keys_fail_closed_until_expired_keys_are_swept() {
        let t = AuthThrottle::with_capacity_for_test(2);
        assert!(t.check("k1", 1_000, 5));
        assert!(t.check("k2", 1_000, 5));
        // Map full, new key, nothing expired → fail closed (throttled).
        assert!(!t.check("k3", 1_500, 5));
        // 61s later k1/k2's hits are expired; the sweep frees room.
        assert!(t.check("k3", 62_000, 5));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml http::throttle`
Expected: FAIL (module does not exist / does not compile).

- [ ] **Step 3: Implement**

```rust
//! Shared sliding-window throttle for the two Argon2-verifying auth endpoints
//! (`/api/login`, `/api/invites/accept`). Keys are opaque strings composed by
//! the callers (`login:u:<username>`, `login:ip:<ip>`, `invite:u:<uuid>`,
//! `invite:ip:<ip>`). INVARIANT (no enumeration oracle): identity keys count
//! attempts against unknown identities exactly like known ones, and callers
//! return one uniform 429 — the throttle must never behave differently for an
//! identity that exists.

use std::collections::HashMap;
use std::sync::Mutex;

/// Per-identity login budget (trailing 60 s). Bounds targeted credential
/// stuffing on one account while leaving interactive retry usable.
pub const LOGIN_PER_MIN_PER_IDENTITY: usize = 10;
/// Per-IP login budget — bounds identity-rotating stuffing from one address.
pub const LOGIN_PER_MIN_PER_IP: usize = 30;
/// Per-account invite-redemption budget (the caller is authenticated).
pub const INVITE_PER_MIN_PER_ACCOUNT: usize = 10;
/// Per-IP invite-redemption budget.
pub const INVITE_PER_MIN_PER_IP: usize = 30;
/// Tracked-key capacity. Unauthenticated input mints identity keys, so the map
/// must be bounded; per-IP budgets cap the mint rate per address, and on
/// overflow new keys FAIL CLOSED (throttled) rather than evicting live state.
pub const MAX_TRACKED_KEYS: usize = 65_536;

const WINDOW_MS: i64 = 60_000;

pub struct AuthThrottle {
    hits: Mutex<HashMap<String, Vec<i64>>>,
    capacity: usize,
}

impl AuthThrottle {
    pub fn new() -> Self {
        Self { hits: Mutex::new(HashMap::new()), capacity: MAX_TRACKED_KEYS }
    }

    #[cfg(test)]
    pub(crate) fn with_capacity_for_test(capacity: usize) -> Self {
        Self { hits: Mutex::new(HashMap::new()), capacity }
    }

    /// Record an attempt under `key` at `now_ms`; `true` iff within `per_min`
    /// over the trailing 60 s. At capacity, expired keys are swept first; if
    /// the map is still full a NEW key is refused (fail closed).
    pub fn check(&self, key: &str, now_ms: i64, per_min: usize) -> bool {
        let cutoff = now_ms - WINDOW_MS;
        let mut map = self.hits.lock().expect("auth-throttle mutex poisoned");
        if !map.contains_key(key) && map.len() >= self.capacity {
            map.retain(|_, v| v.iter().any(|&t| t > cutoff));
            if map.len() >= self.capacity {
                return false;
            }
        }
        let v = map.entry(key.to_owned()).or_default();
        v.retain(|&t| t > cutoff);
        if v.len() >= per_min {
            return false;
        }
        v.push(now_ms);
        true
    }
}

impl Default for AuthThrottle {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Wire the state field**

In `http/mod.rs`: add `pub mod throttle;` beside the existing module declarations, add
`pub auth_throttle: Arc<throttle::AuthThrottle>,` to `AppState` (mod.rs:21-28), and
`auth_throttle: Arc::new(throttle::AuthThrottle::new()),` to `tests::test_state`
(mod.rs:173-183). Mirror the field in `main.rs:50-57` and `bin/test_server.rs:156-163`
(`Arc::new(shadowcat::http::throttle::AuthThrottle::new())`).

- [ ] **Step 5: Run to verify pass, then commit**

Run: `cargo test --manifest-path src/server/Cargo.toml http::throttle`
Expected: 3 PASS; whole suite compiles.

```bash
git add src/server/src/http/throttle.rs src/server/src/http/mod.rs src/server/src/main.rs src/server/src/bin/test_server.rs
git commit -m "feat(server): shared AuthThrottle sliding-window limiter"
```

---

### Task 2: Throttle `/api/login` + `/api/invites/accept`, plumb client IP

**Files:**
- Modify: `src/server/src/http/throttle.rs` (add `ClientIp` extractor)
- Modify: `src/server/src/http/routes.rs` (`login` at :139-182, `accept_invite` at :671-717)
- Modify: `src/server/src/main.rs:62-64`, `src/server/src/bin/test_server.rs` (serve with connect-info)
- Test: `src/server/src/http/mod.rs` `tests` module

**Interfaces:**
- Consumes: Task 1's `AuthThrottle`, constants, `AppState.auth_throttle`.
- Produces: `throttle::ClientIp(pub Option<std::net::IpAddr>)` axum extractor.

- [ ] **Step 1: Write the failing tests** (in `http/mod.rs` `tests`, following the
  `login_rejects_wrong_password_and_unknown_user_identically` convention at mod.rs:405)

```rust
#[tokio::test]
async fn login_throttles_identity_after_budget_spending_no_argon2() {
    use crate::auth::password::verify_count;
    use crate::http::throttle::LOGIN_PER_MIN_PER_IDENTITY;
    let server = server_with_user("gm-1", "pw-correct", ServerRole::User).await;

    // Unknown identity: exhaust the budget, then assert 429 + zero verifies.
    for _ in 0..LOGIN_PER_MIN_PER_IDENTITY {
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "ghost", "password": "x" }))
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }
    let before = verify_count();
    let ghost_throttled = server
        .post("/api/login")
        .json(&serde_json::json!({ "username": "ghost", "password": "x" }))
        .await;
    ghost_throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(verify_count() - before, 0, "throttled attempt must spend no Argon2");

    // Known identity: identical throttle shape (status AND body) — no oracle.
    for _ in 0..LOGIN_PER_MIN_PER_IDENTITY {
        server
            .post("/api/login")
            .json(&serde_json::json!({ "username": "gm-1", "password": "wrong" }))
            .await
            .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    }
    let known_throttled = server
        .post("/api/login")
        .json(&serde_json::json!({ "username": "gm-1", "password": "wrong" }))
        .await;
    known_throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(ghost_throttled.text(), known_throttled.text(), "uniform 429 body");
}

#[tokio::test]
async fn accept_invite_throttles_per_account_spending_no_argon2() {
    use crate::auth::password::verify_count;
    use crate::http::throttle::INVITE_PER_MIN_PER_ACCOUNT;
    let f = invite_fixture().await; // existing fixture, mod.rs:955-995
    for _ in 0..INVITE_PER_MIN_PER_ACCOUNT {
        f.other_gm
            .post("/api/invites/accept")
            .json(&serde_json::json!({ "code": "not-a-real-code" }))
            .await
            .assert_status(axum::http::StatusCode::NOT_FOUND);
    }
    let before = verify_count();
    let throttled = f
        .other_gm
        .post("/api/invites/accept")
        .json(&serde_json::json!({ "code": "not-a-real-code" }))
        .await;
    throttled.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(verify_count() - before, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml login_throttles accept_invite_throttles`
Expected: FAIL (429 never returned).

- [ ] **Step 3: Implement the extractor** (append to `throttle.rs`)

```rust
/// Infallible client-IP extractor: `Some` when the server is served with
/// connect-info (production `main.rs`), `None` under the axum-test mock
/// transport — IP throttling degrades to identity-only there, never a 500.
pub struct ClientIp(pub Option<std::net::IpAddr>);

impl<S> axum::extract::FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|ci| ci.0.ip()),
        ))
    }
}
```

- [ ] **Step 4: Implement handler wiring**

In `routes.rs::login` (:141-145): add `ip: crate::http::throttle::ClientIp` as a parameter
**before** `Json(body)`, and insert at the top of the body (before the `user_by_username`
lookup, so a throttled request costs neither a DB hit nor a verify):

```rust
    use crate::http::throttle::{LOGIN_PER_MIN_PER_IDENTITY, LOGIN_PER_MIN_PER_IP};
    let now = now_millis();
    // Identity key counts unknown usernames identically to known ones — the
    // throttle must not become the enumeration oracle the constant-verify
    // design below exists to prevent. Uniform 429 for both key kinds.
    let ident_key = format!("login:u:{}", body.username.to_lowercase());
    let ip_ok = match ip.0 {
        Some(addr) => state
            .auth_throttle
            .check(&format!("login:ip:{addr}"), now, LOGIN_PER_MIN_PER_IP),
        None => true,
    };
    if !ip_ok || !state.auth_throttle.check(&ident_key, now, LOGIN_PER_MIN_PER_IDENTITY) {
        return Err(AppError::TooManyRequests("try again later".into()));
    }
```

In `routes.rs::accept_invite` (:671-675): add `ip: crate::http::throttle::ClientIp` before
`Json(body)`, and insert at the top of the body:

```rust
    use crate::http::throttle::{INVITE_PER_MIN_PER_ACCOUNT, INVITE_PER_MIN_PER_IP};
    let now = now_millis();
    let ip_ok = match ip.0 {
        Some(addr) => state
            .auth_throttle
            .check(&format!("invite:ip:{addr}"), now, INVITE_PER_MIN_PER_IP),
        None => true,
    };
    if !ip_ok
        || !state
            .auth_throttle
            .check(&format!("invite:u:{}", user.id), now, INVITE_PER_MIN_PER_ACCOUNT)
    {
        return Err(AppError::TooManyRequests("try again later".into()));
    }
```

Note: the IP check runs first in both handlers so an identity key is only minted for
requests inside the IP budget (bounds map growth per address).

In `main.rs:64`: `axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;`
Apply the same change to `bin/test_server.rs`'s serve call.

- [ ] **Step 5: Run full suite, fix any budget-tripping tests, commit**

Run: `cargo test --manifest-path src/server/Cargo.toml`
Expected: PASS. If a pre-existing test trips a budget (it repeats one identity > 10×/min in
one fixture), give that test distinct identities — never raise the constants.

```bash
git add src/server/src/http/throttle.rs src/server/src/http/routes.rs src/server/src/http/mod.rs src/server/src/main.rs src/server/src/bin/test_server.rs
git commit -m "feat(server): throttle /api/login and /api/invites/accept (identity + IP keys)"
```

---

### Task 3: Garbage-collect spent `world_invites` rows in the session sweep

**Files:**
- Modify: `src/server/src/auth/session.rs:129-149` (`spawn_session_sweep`)
- Test: `src/server/src/auth/session.rs` tests

**Interfaces:**
- Produces: `pub(crate) async fn sweep_spent_invites(pool: &sqlx::SqlitePool, now_ms: i64) -> Result<u64, sqlx::Error>`; `const INVITE_GC_GRACE_MS: i64`.
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn sweep_deletes_only_rows_expired_past_grace() {
    use crate::data::sqlite::{NewInvite, SqliteRepository};
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo.create_user("gm", None, crate::auth::role::ServerRole::User, 0).await.unwrap();
    let world = repo.create_world_owned("W", gm, 0).await.unwrap();
    let now: i64 = 100 * 24 * 60 * 60 * 1000; // day 100
    let mk = |id: u128, expires_at: i64| NewInvite {
        id: uuid::Uuid::from_u128(id),
        world: world.id,
        secret_hash: "x",
        role: crate::data::document::WorldRole::Player,
        created_by: gm,
        now: 0,
        expires_at,
    };
    // Expired 31 days ago → swept. Expired 1 day ago → kept (inside grace).
    repo.create_invite(mk(1, now - 31 * 24 * 60 * 60 * 1000), 64).await.unwrap();
    repo.create_invite(mk(2, now - 1 * 24 * 60 * 60 * 1000), 64).await.unwrap();
    let deleted = super::sweep_spent_invites(repo.pool(), now).await.unwrap();
    assert_eq!(deleted, 1);
    assert!(repo.invite_by_id(uuid::Uuid::from_u128(1)).await.unwrap().is_none());
    assert!(repo.invite_by_id(uuid::Uuid::from_u128(2)).await.unwrap().is_some());
}
```

(If `NewInvite`'s field names differ, match the real struct at `data/sqlite.rs` —
`create_invite` is called with it at `routes.rs:596-607`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml sweep_deletes_only_rows_expired_past_grace`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement**

```rust
/// Retention past `expires_at` before a spent invite row is deleted. Consumed
/// and revoked rows also age out through this: every row carries the 7-day
/// mint TTL, so all spent rows are gone within TTL + grace. 30 days keeps
/// recent redemption provenance (`consumed_by`) inspectable for a while
/// without unbounded growth.
const INVITE_GC_GRACE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Delete invite rows whose `expires_at` is more than the grace period past.
/// Correctness does not depend on this: expired rows are already unredeemable
/// (`consume_invite`'s guarded UPDATE); this bounds table growth.
pub(crate) async fn sweep_spent_invites(
    pool: &sqlx::SqlitePool,
    now_ms: i64,
) -> Result<u64, sqlx::Error> {
    let cutoff = now_ms - INVITE_GC_GRACE_MS;
    let res = sqlx::query("DELETE FROM world_invites WHERE expires_at <= ?")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
```

In `spawn_session_sweep` (session.rs:138-149), clone the pool alongside the store and add
after the `delete_expired` call inside the loop:

```rust
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            if let Err(e) = sweep_spent_invites(&pool, now).await {
                tracing::warn!(error = %e, "invite sweep failed");
            }
```

(`let pool = repo.pool().clone();` before the `tokio::spawn`.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --manifest-path src/server/Cargo.toml auth::session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/auth/session.rs
git commit -m "feat(server): GC spent world_invites rows in the session sweep"
```

---

### Task 4: `TokenEngine` coordinate validation at ingress

**Files:**
- Modify: `src/server/src/data/engine/token.rs` (add `validate`)
- Modify: `src/server/src/data/engine/mod.rs:111-131` (`normalize_engine`'s `"token"` arm)
- Test: `src/server/src/data/engine/token.rs` + an anti-drift test in `src/server/src/data/validation.rs` tests

**Interfaces:**
- Consumes: `crate::scene::move_exec::MAX_GATE_WALK_COORD` (`pub(crate)`, move_exec.rs:74) — the ONE shared bound symbol.
- Produces: `TokenEngine::validate(&self) -> Result<(), String>`.

- [ ] **Step 1: Write the failing tests** (in `token.rs`, new `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> TokenEngine {
        TokenEngine {
            x: 0.0, y: 0.0, w: 100.0, h: 100.0, rotation: 0.0,
            visual: None, actor_id: None, overrides: None, face: None,
        }
    }

    #[test]
    fn finite_in_bound_token_validates() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn non_finite_fields_are_rejected() {
        for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut t = base();
            t.x = f;
            assert!(t.validate().is_err(), "x = {f} must be rejected");
            let mut t = base();
            t.rotation = f;
            assert!(t.validate().is_err(), "rotation = {f} must be rejected");
        }
    }

    #[test]
    fn ingress_bound_equals_gate_walks_exactly() {
        // Anti-drift: ingress and the movement gate read ONE symbol with the
        // same strictly-`>` sense (template: room.rs's
        // publish_move_gate_admissibility_bound_equals_gate_walks).
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        let mut t = base();
        t.x = bound;
        assert!(t.validate().is_ok(), "AT the bound is admissible");
        t.x = bound + 1.0;
        assert!(t.validate().is_err(), "over the bound is refused");
        let mut t = base();
        t.y = -(bound + 1.0);
        assert!(t.validate().is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml data::engine::token`
Expected: FAIL (no `validate` method).

- [ ] **Step 3: Implement `validate` + wire the seam**

In `token.rs`:

```rust
impl TokenEngine {
    /// Ingress validation beyond serde shape: every numeric field finite, and
    /// the position inside the ONE shared movement-coordinate bound
    /// (`scene::move_exec::MAX_GATE_WALK_COORD`) — the GM-write/Create path
    /// and the move gate must agree on admissible coordinates structurally,
    /// never by call ordering.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, v) in [
            ("x", self.x),
            ("y", self.y),
            ("w", self.w),
            ("h", self.h),
            ("rotation", self.rotation),
        ] {
            if !v.is_finite() {
                return Err(format!("{name} must be finite"));
            }
        }
        let bound = crate::scene::move_exec::MAX_GATE_WALK_COORD;
        if self.x.abs() > bound || self.y.abs() > bound {
            return Err(format!("position exceeds coordinate bound {bound}"));
        }
        Ok(())
    }
}
```

In `engine/mod.rs:112`, replace the `"token"` arm:

```rust
        "token" => {
            let typed: TokenEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("token: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("token: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
```

- [ ] **Step 4: Add the chokepoint test** (in `validation.rs` tests, beside
  `validate_engine_tree_rejects_malformed_engine_body` at :545)

```rust
    #[test]
    fn validate_engine_tree_rejects_out_of_bound_token_position() {
        let over = crate::scene::move_exec::MAX_GATE_WALK_COORD + 1.0;
        let mut doc = doc_with_engine(serde_json::json!({
            "x": over, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0
        }));
        doc.doc_type = "token".into();
        assert!(matches!(
            validate_engine_tree(&mut doc),
            Err(DataError::BadEngine(_))
        ));
    }
```

(Adapt `doc_with_engine`'s doc_type handling to the real helper — it may take the type as an
argument; keep the assertion identical.)

- [ ] **Step 5: Run full suite, commit**

Run: `cargo test --manifest-path src/server/Cargo.toml`
Expected: PASS — if any existing fixture seeds a token beyond 1e9 or with non-finite fields,
fix the fixture (pre-v1, no stored-data migration concerns).

```bash
git add src/server/src/data/engine/token.rs src/server/src/data/engine/mod.rs src/server/src/data/validation.rs
git commit -m "feat(server): validate TokenEngine coordinates at ingress (shared gate bound)"
```

---

### Task 5: Remove the five `unwrap_or(100.0)` survivors in `scene/mod.rs`

**Files:**
- Modify: `src/server/src/scene/mod.rs` — `navmesh_for` (:1169-1173), `region_field` (:1362-1367), `player_lit_mask` (:1742), `visible_cells` (:1894-1898), `visible_cells_cached` (:1938-1942)
- Modify: `src/server/src/scene/move_exec.rs:304` (+ `MoveReject` enum) and `scene/mod.rs:1248,1268` (`region_field` callers)
- Test: `src/server/src/scene/mod.rs` tests

**Interfaces:**
- Produces: `region_field(&self, scene: Uuid, viewer: Option<Uuid>) -> Option<regions::RegionField>` (signature change); new `MoveReject::SceneUnknown` variant.
- Consumes: nothing from other tasks. `scene_grid_sizes` (:1046) stays the intentional defaulting source; its invariant — an entry exists for every live scene, so an absent entry means "scene has no document" — is what every fix below keys on.

- [ ] **Step 1: Write the failing tests** (in `scene/mod.rs` tests, using the existing
  `scene_with_lit_player_token` fixture at :4437 and builders at :2462/:2601)

```rust
    #[test]
    fn absent_scene_yields_empty_visible_cells_not_a_synthesized_grid() {
        let (ecs, user, _scene) = scene_with_lit_player_token();
        let ghost_scene = Uuid::from_u128(0xDEAD);
        assert!(ecs.visible_cells(user, ghost_scene, false).is_empty());
        assert!(ecs.visible_cells(user, ghost_scene, true).is_empty());
        assert!(ecs.visible_cells_cached(user, ghost_scene, false).is_empty());
    }

    #[test]
    fn absent_scene_region_field_is_none() {
        let (ecs, _user, _scene) = scene_with_lit_player_token();
        assert!(ecs.region_field(Uuid::from_u128(0xDEAD), None).is_none());
    }

    #[test]
    fn absent_scene_navmesh_for_is_none() {
        let (ecs, _user, _scene) = scene_with_lit_player_token();
        assert!(ecs.navmesh_for(Uuid::from_u128(0xDEAD), 0.5).is_none());
    }
```

(`player_lit_mask` iterates scenes discovered from vision sources, so an absent-scene entry
cannot be injected from outside; its fix is the same `let-else` skip and is covered by
compilation + the existing 13 mask tests staying green.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml absent_scene`
Expected: `region_field` test fails to compile (returns non-Option) — that is the failure
signal; the others may already pass via downstream emptiness, keep them as pins.

- [ ] **Step 3: Implement — one shape at every site**

`navmesh_for` (:1169-1173) — the function already returns `Option`:

```rust
        let cell = self.scene_grid_sizes().get(&scene).copied()?;
```

`region_field` — change the signature to `-> Option<regions::RegionField>`, replace
:1363-1367 with:

```rust
        let cell = self.scene_grid_sizes().get(&scene).copied()?;
```

and wrap the final builder return in `Some(...)`. Update its doc comment: absent scene ⇒
`None`; callers must refuse, mirroring `pathfind`'s `PathFail::Invalid`.

`region_field` callers:
- `scene/mod.rs:1248` and `:1268` (both in `pathfind`):
  ```rust
          let Some(regions) = self.region_field(scene, if is_gm { None } else { Some(user) })
          else {
              return Err(pathfinding::PathFail::Invalid);
          };
  ```
- `scene/move_exec.rs:304` (in `execute_move`):
  ```rust
      let Some(regions) = ecs.region_field(scene, None) else {
          return Err(MoveReject::SceneUnknown);
      };
  ```
  Add the variant to `MoveReject` beside `Degenerate` with doc comment
  `/// The token's scene has no document — refuse rather than synthesize a grid.`
  and mirror `Degenerate`'s handling at every `match` over `MoveReject`
  (`Room::execute_move` maps rejects to `DataError`/`MoveError`; give `SceneUnknown` the
  same arm as `Degenerate`).
- Test callers (`scene/mod.rs:5024, 5031, 5069, 5469, 5703`; `move_exec.rs:1002, 1068`):
  append `.expect("scene exists")` at each.

`player_lit_mask` (:1742) — per-scene skip, matching the `None => continue` precedent
directly above it at :1738-1741:

```rust
            let Some(cell) = grid.get(&scene).copied() else {
                continue;
            };
```

`visible_cells` (:1894-1898):

```rust
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return out;
        };
```

`visible_cells_cached` (:1938-1942):

```rust
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return BTreeSet::new();
        };
```

- [ ] **Step 4: Run full suite**

Run: `cargo test --manifest-path src/server/Cargo.toml`
Expected: PASS (the three gate call sites — `publish` :303, `execute_move` :546,
`pathfind` :1212 — already refuse absent scenes above these functions, so behavior is
unchanged on every live path; what changed is that the agreement is now structural).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/scene/mod.rs src/server/src/scene/move_exec.rs
git commit -m "fix(server): absent-scene is explicit in the five scene/mod.rs cell-size sites"
```

---

### Task 6: Remove the `unwrap_or(100.0)` + `SquareGrid` fallback in `enrich_vision_explored`

**Files:**
- Modify: `src/server/src/ws/conn.rs:740-766`
- Test: `src/server/src/ws/conn.rs` tests (beside `enrich_accumulates_persists_and_emits_explored` at :1843)

**Interfaces:**
- Consumes: nothing from other tasks. The two fallbacks at conn.rs:742 and :747-756 are
  documented as mirrors of each other and must be removed together.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn enrich_skips_scene_absent_from_grid_maps() {
        // A masked vision payload naming a scene that is missing from BOTH
        // grid maps must contribute no explored entry (fail closed), not be
        // indexed against a synthesized 100-unit square grid.
        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let world = Uuid::from_u128(1);
        let user = Uuid::from_u128(2);
        let ghost = Uuid::from_u128(0xDEAD);
        let mut payload = serde_json::json!({
            "mode": "masked",
            "polygons": [{ "scene": ghost, "points": [0.0, 0.0, 200.0, 0.0, 200.0, 200.0] }]
        });
        let grid: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
        let shapes = square_grid_shapes(&grid); // existing helper, conn.rs:1821
        enrich_vision_explored(&mut payload, &grid, &shapes, repo.as_ref(), world, user, true)
            .await;
        let explored = payload.get("explored").and_then(|e| e.as_array());
        assert!(
            explored.map(|a| a.is_empty()).unwrap_or(true),
            "no explored entry for a scene with no grid entry"
        );
    }
```

(Match the payload shape to what the existing enrich tests at :1843/:2469 build — copy their
polygon fixture shape exactly.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml enrich_skips_scene_absent`
Expected: FAIL — today the fallback synthesizes `cell = 100.0` + a square grid and emits an
explored entry.

- [ ] **Step 3: Implement**

Replace conn.rs:741-756 (the `unwrap_or(100.0)`, the `fallback` `SquareGrid`, and the
`.unwrap_or(&fallback)`) with:

```rust
        // A scene absent from either map has no live scene document — skip it
        // (fail closed: the client masks everything outside `polygons`, so a
        // skipped scene simply contributes no explored). Never synthesize a
        // grid no scene declared.
        let Some(cell) = grid.get(&scene).copied() else {
            continue;
        };
        let Some(shape) = grid_shapes.get(&scene).map(|b| b.as_ref()) else {
            continue;
        };
```

(`shape` keeps its `&(dyn GridShape + Send + Sync)` type from the map's box — the
`+ Send + Sync` bound note at :751-752 still applies; delete the now-stale fallback comment.)

- [ ] **Step 4: Run the conn.rs suite**

Run: `cargo test --manifest-path src/server/Cargo.toml ws::conn`
Expected: PASS including all 5 existing enrich tests.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/conn.rs
git commit -m "fix(server): enrich_vision_explored skips scenes with no grid entry"
```

---

### Task 7: `ScenePing` guard — scene exists in this world + `cap::READ`

**Files:**
- Modify: `src/server/src/ws/conn.rs:354-366` (extract + guard) and `src/server/src/ws/protocol.rs:63-66` (doc comment)
- Test: `src/server/src/ws/conn.rs` tests

**Interfaces:**
- Produces: `async fn scene_ping_permitted(scene: Uuid, ctx: &PermissionContext, world_id: Uuid, repo: &dyn Repository) -> bool` in conn.rs.
- Consumes: `resolve_access_world` / `cap::READ` (data/permission.rs:400), `world_cap_defaults` (as used at routes.rs:377).

- [ ] **Step 1: Write the failing tests** (template: `pathfind_handler_gm_ok_nongm_dark_unreachable` at conn.rs:1955 — same repo/world/room/PermissionContext scaffolding)

```rust
    #[tokio::test]
    async fn scene_ping_guard_admits_reader_refuses_foreign_and_hidden() {
        use crate::data::document::{Visibility, WorldRole};
        use crate::data::membership::PermissionContext;
        let repo = Arc::new(SqliteRepository::connect("sqlite::memory:").await.unwrap());
        let gm = repo.create_user("gm", None, crate::auth::role::ServerRole::User, 0).await.unwrap();
        let world = repo.create_world_owned("W", gm, 0).await.unwrap();
        let other_world = repo.create_world_owned("X", gm, 0).await.unwrap();
        let p = repo.create_user("player", None, crate::auth::role::ServerRole::User, 0).await.unwrap();
        repo.add_member(world.id, p, WorldRole::Spectator).await.unwrap();
        let spectator = PermissionContext { user_id: p, world_role: WorldRole::Spectator };
        let gm_ctx = PermissionContext { user_id: gm, world_role: WorldRole::Gm };

        let wdoc = crate::data::document::tests::world_scoped_doc;
        // A readable scene in this world; a scene in the other world; a scene
        // whose permissions.default denies READ.
        let (vis_id, foreign_id, hidden_id) =
            (Uuid::from_u128(0xB001), Uuid::from_u128(0xB002), Uuid::from_u128(0xB003));
        let mut vis = wdoc(world.id, vis_id, "scene");
        vis.owner = Some(gm);
        let mut foreign = wdoc(other_world.id, foreign_id, "scene");
        foreign.owner = Some(gm);
        let mut hidden = wdoc(world.id, hidden_id, "scene");
        hidden.owner = Some(gm);
        hidden.permissions.default = crate::data::document::DocRole::None;
        for d in [vis, foreign, hidden] {
            repo.apply_intent(&gm_ctx_for(&repo, d.world_id.unwrap_or(world.id), gm).await,
                d.world_id.unwrap_or(world.id),
                vec![crate::data::command::Operation::Create { doc: d.clone() }],
                0, WriteOrigin::Client).await.unwrap();
        }

        // Token-less spectator may ping the scene they can read...
        assert!(scene_ping_permitted(vis_id, &spectator, world.id, repo.as_ref()).await);
        // ...but not a scene in another world, a hidden scene, or a ghost id.
        assert!(!scene_ping_permitted(foreign_id, &spectator, world.id, repo.as_ref()).await);
        assert!(!scene_ping_permitted(hidden_id, &spectator, world.id, repo.as_ref()).await);
        assert!(!scene_ping_permitted(Uuid::from_u128(0xDEAD), &spectator, world.id, repo.as_ref()).await);
    }
```

(Adapt doc-seeding to the conventions the pathfind test actually uses — `room.publish` with a
GM ctx is the established path, see conn.rs:2016-2038; the `world_scoped_doc` helper carries
the world id. Field names for the deny-READ default must match `data/document.rs` —
`permissions.default` holding a `DocRole`; verify against `buildTokenDoc`'s server mirror.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml scene_ping_guard`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement**

In conn.rs (beside `handle_pathfind` at :515):

```rust
/// Whether `ctx` may ping into `scene`: the doc must exist, be a scene, belong
/// to THIS world, and grant the sender `cap::READ`. Admits a token-less
/// spectator (READ on the scene is enough — deliberately weaker than
/// `handle_pathfind`'s controls-a-token gate, which selects server state;
/// ping selects none). Denial is a SILENT drop: any error frame or behavior
/// split would leak scene existence to a non-reader.
async fn scene_ping_permitted(
    scene: Uuid,
    ctx: &crate::data::membership::PermissionContext,
    world_id: Uuid,
    repo: &dyn crate::data::repository::Repository,
) -> bool {
    let Ok(Some(doc)) = repo.get_document(scene).await else {
        return false;
    };
    if doc.doc_type != "scene" {
        return false;
    }
    // World scope: a scene doc from another world is refused even for a
    // member of both (the relay stamps THIS room).
    if crate::data::document::world_of(&doc) != Some(world_id) {
        return false;
    }
    let Ok(defaults) = repo.world_cap_defaults(world_id).await else {
        return false;
    };
    let access = crate::data::permission::resolve_access_world(
        ctx.user_id,
        ctx.world_role,
        &doc,
        &defaults.grants_for(&doc.doc_type),
    );
    access.has(crate::data::permission::cap::READ)
}
```

(`world_of`: routes.rs:790 uses a `world_of(&doc)` helper — reuse it if it is importable from
there, else read the document's world field directly the way that helper does. If
`world_cap_defaults` is not on the `Repository` trait, take `repo: &SqliteRepository` exactly
as `enrich_vision_explored` (:709) already does.)

Rewire the arm at :354-366:

```rust
                        Ok(ClientMsg::ScenePing { scene, x, y }) => {
                            // Guard order: cheap rate check first, then the
                            // authz lookup (one doc read per admitted ping,
                            // bounded at 30/min/user). Over-budget and
                            // unauthorized pings both drop silently.
                            if ping_rate.check(user_id, now_millis(), 30)
                                && scene_ping_permitted(scene, &ctx, world_id, repo.as_ref()).await
                            {
                                room.broadcast_aux(ServerMsg::ScenePing {
                                    scene,
                                    x,
                                    y,
                                    user: user_id,
                                });
                            }
                        }
```

Update the protocol doc comment at protocol.rs:63-66: replace "Coordinates are not validated
(#6)" with "Coordinates are not validated; the scene must exist in this world and grant the
sender READ (silent drop otherwise)".

- [ ] **Step 4: Run the suite**

Run: `cargo test --manifest-path src/server/Cargo.toml ws::conn`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/ws/conn.rs src/server/src/ws/protocol.rs
git commit -m "fix(server): ScenePing requires an in-world scene the sender can READ"
```

---

### Task 8: `handle_send_message` — pin the actor doc's world scope

**Files:**
- Modify: `src/server/src/chat/mod.rs:500-517`
- Test: `src/server/src/chat/mod.rs` tests (beside the `ActorNotSpeakable` tests at :2697)

**Interfaces:**
- Consumes: `room.world_id` (already read later in the same function at :605); the same
  world-scope read used by Task 7.

- [ ] **Step 1: Write the failing test** (copy the scaffolding of the existing
  `ActorNotSpeakable` test at chat/mod.rs:2697 — same room/repo/actor seeding — then:)

```rust
    #[tokio::test]
    async fn actor_from_another_world_is_not_speakable_even_for_its_owner() {
        // Seed TWO worlds; the actor doc lives in world B, the send targets
        // world A's room, the sender owns the actor. Expect ActorNotSpeakable:
        // cross-world attribution is refused even though ownership matches.
        // (Build exactly as the existing ActorNotSpeakable tests do, with the
        // actor's Create published into the OTHER world's room.)
        // ... fixture per chat/mod.rs:2697 conventions ...
        let err = crate::chat::handle_send_message(
            &room_a, repo.as_ref(), &ctx_owner, &rate, deps(),
            "all".into(), "hi".into(),
            Some(ActorOwnerRef::Actor { actor_id: actor_in_world_b }),
            Audience::Public, 0, 30,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SendMessageError::ActorNotSpeakable));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml actor_from_another_world`
Expected: FAIL — today the cross-world actor passes the check (owner matches).

- [ ] **Step 3: Implement**

At chat/mod.rs:508-513, extend the `allowed` match to require the actor doc's world to be
THIS room's world:

```rust
                let allowed = match &actor_doc {
                    // GM may attribute as any actor doc IN THIS WORLD; a
                    // Player only as one they own. The world pin closes the
                    // cross-world ref shape (previously inert — foreign refs
                    // failed closed to no attribution client-side — now
                    // refused at ingest like every other invalid ref).
                    Some(d)
                        if d.doc_type == "actor"
                            && crate::data::document::world_of(d) == Some(room.world_id) =>
                    {
                        is_gm || d.owner == Some(ctx.user_id)
                    }
                    _ => false,
                };
```

(Use the same world-scope accessor Task 7 settled on.)

- [ ] **Step 4: Run the chat suite**

Run: `cargo test --manifest-path src/server/Cargo.toml chat::`
Expected: PASS including the 4 existing `ActorNotSpeakable` tests.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/mod.rs
git commit -m "fix(server): handle_send_message pins actor attribution to the sending world"
```

---

### Task 9: `apply_command` gains the `/engine` normalization gate

**Files:**
- Modify: `src/server/src/data/sqlite.rs:1325-1423` (`apply_command`)
- Test: `src/server/src/data/sqlite.rs` tests (mirror :3573 and :3714)

**Interfaces:**
- Consumes: `validation::validate_engine_tree` (validation.rs:73) and the normalization shape
  from `apply_intent` (sqlite.rs:1798-1894).

- [ ] **Step 1: Write the failing tests** — copy
  `apply_intent_update_normalizes_engine_broadcast_and_event_log_smuggled_key` (:3573) and
  `apply_intent_update_normalizes_engine_integer_literal_to_stored_float` (:3714), rename with
  an `apply_command_` prefix, and drive the same ops through `repo.apply_command(UnsequencedCommand { ... })`
  instead of `apply_intent` (no ctx — build `UnsequencedCommand` with the same world/author/ts/ops;
  its shape is visible at the existing `apply_command` test call sites past sqlite.rs:2269).
  Assert: (a) the stored row holds the normalized engine value, (b) the returned `Command`'s
  `FieldChange.new` equals the normalized value (not the raw submitted JSON), (c) a malformed
  engine body returns `Err(DataError::BadEngine(_))`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml apply_command_update_normalizes`
Expected: FAIL — apply_command stores the raw un-normalized value.

- [ ] **Step 3: Implement**

In `apply_command`'s Update arm (sqlite.rs:1381-1408), after `check_command_scope(&doc, ...)`
and before `doc.updated_at = sequenced.ts;`, insert:

```rust
                    // Same /engine gate as apply_intent (the trusted substrate
                    // skips capability/schema/size checks by design, but the
                    // engine band's normalize-then-store invariant is data
                    // integrity, not authz — the row, the log, and any future
                    // replay must carry the identical normalized value).
                    crate::data::validation::validate_engine_tree(&mut doc)?;
```

Then rebuild the op exactly as apply_intent does: collect into a `normalized_ops` vec —
restructure the apply loop to push each op (Create/Delete arms push `op.clone()` unchanged;
the Update arm re-derives `/engine`-prefixed `FieldChange.new` from the normalized doc):

```rust
                    let normalized_doc_json = serde_json::to_value(&doc)?;
                    let normalized_changes: Vec<FieldChange> = changes
                        .iter()
                        .map(|ch| {
                            if ch.path == "/engine" || ch.path.starts_with("/engine/") {
                                if let Some(v) = normalized_doc_json.pointer(&ch.path) {
                                    return FieldChange {
                                        remove: false,
                                        path: ch.path.clone(),
                                        old: ch.old.clone(),
                                        new: v.clone(),
                                    };
                                }
                            }
                            ch.clone()
                        })
                        .collect();
                    normalized_ops.push(Operation::Update {
                        doc_id: *doc_id,
                        changes: normalized_changes,
                    });
```

Also gate the Create arm (apply_intent validates Creates before its Phase 2; apply_command
must not store an unvalidated engine body either):

```rust
                Operation::Create { doc } => {
                    check_command_scope(doc, sequenced.world_id)?;
                    let mut doc = doc.clone();
                    crate::data::validation::validate_engine_tree(&mut doc)?;
                    Self::upsert_document(&mut tx, &doc, seq).await?;
                    normalized_ops.push(Operation::Create { doc });
                }
```

After the loop: `sequenced.ops = normalized_ops;` before the `world_events` INSERT, so the
log carries normalized ops (identical to apply_intent's ordering).

- [ ] **Step 4: Run the suite**

Run: `cargo test --manifest-path src/server/Cargo.toml data::sqlite`
Expected: PASS (existing apply_command tests must still pass — they use well-formed docs).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "fix(server): apply_command mirrors apply_intent's /engine normalization gate"
```

---

### Task 10: `ReplaceDie` Faces gate

**Files:**
- Modify: `src/server/src/dice/recalc.rs:62-68` (+ `RecalcOp` doc comment :7-12)
- Test: `src/server/src/dice/recalc.rs` tests

**Interfaces:**
- Consumes: `RawDie.kind` (each raw die carries its own `DieKind` — see the `Faces` fixture
  at recalc.rs:230-236).

- [ ] **Step 1: Write the failing tests** (using the existing `Faces` fixture shape from
  `reroll_dice_redraws_a_fresh_index_for_a_faces_die` at recalc.rs:201-251)

```rust
    #[test]
    fn replace_die_out_of_range_index_on_faces_die_is_skipped() {
        // Build the 2-face fixture exactly as recalc.rs:201-251 does
        // (faces [value 1, value 6], one die, natural = 0).
        // ... same spec/naturals/raws construction ...
        let mut rng = ScriptedRng::new(vec![]);
        // Index 2 is out of range for a 2-face die → op skipped, identity.
        let (r1, out1) =
            recalculate(&spec, &raws, &[RecalcOp::ReplaceDie { id: 0, natural: 2 }], &mut rng);
        assert_eq!(r1.dice[0].natural, 0, "out-of-range replace is ignored");
        assert_eq!(out1.total, 1, "outcome unchanged");
        // Negative index likewise (would wrap to a huge usize at the reader).
        let (r2, _) =
            recalculate(&spec, &raws, &[RecalcOp::ReplaceDie { id: 0, natural: -1 }], &mut rng);
        assert_eq!(r2.dice[0].natural, 0);
        // A VALID index applies.
        let (r3, out3) =
            recalculate(&spec, &raws, &[RecalcOp::ReplaceDie { id: 0, natural: 1 }], &mut rng);
        assert_eq!(r3.dice[0].natural, 1);
        assert_eq!(out3.total, 6);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml replace_die_out_of_range`
Expected: PANIC today (index out of bounds in `face_value_and_symbols`, groups.rs:16) — the
panic IS the bug being fixed.

- [ ] **Step 3: Implement**

Replace the arm at recalc.rs:62-68:

```rust
            RecalcOp::ReplaceDie { id, natural } => {
                for g in groups.iter_mut() {
                    if let Some(d) = g.iter_mut().find(|d| d.id == *id) {
                        match &d.kind {
                            // A Faces natural is a face INDEX consumed by
                            // `faces[natural as usize]` (eval::groups) — an
                            // out-of-range index is ignored like an unknown
                            // id, never written (it would panic at the
                            // reader). Numeric naturals are deliberately
                            // unbounded: out-of-domain replacement is the
                            // GM-override semantic the round-trip test pins.
                            DieKind::Faces { faces } => {
                                if *natural >= 0 && (*natural as usize) < faces.len() {
                                    d.natural = *natural;
                                }
                            }
                            DieKind::Numeric { .. } => {
                                d.natural = *natural;
                            }
                        }
                    }
                }
            }
```

Extend the `RecalcOp` doc comment (:10-12): "…is silently ignored rather than treated as an
error, as is a `ReplaceDie` face index outside a `Faces` die's face list."

- [ ] **Step 4: Run the dice suite** (including proptests)

Run: `cargo test --manifest-path src/server/Cargo.toml dice::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/dice/recalc.rs
git commit -m "fix(dice): ReplaceDie ignores out-of-range face indices on Faces dice"
```

---

### Task 11: Tier-ladder uniqueness guard at the wire boundary

**Files:**
- Modify: `src/server/src/chat/rolls.rs` (`validate_pre_roll` :229-269, `RollError` + Display)
- Test: `src/server/src/chat/rolls.rs` tests

**Interfaces:**
- Produces: `RollError::DuplicateTierOffset(i32)`; `fn validate_tiers(tiers: &[Tier]) -> Result<(), RollError>`.
- Consumes: `Tier` (spec.rs:198-207); `TotalConfig.tiers`/`SuccessConfig.tiers` (spec.rs:240-268).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn duplicate_tier_offsets_are_rejected_pre_roll() {
        use crate::dice::spec::{
            ConstTerm, Direction, Expr, Mode, RollSpec, Tier, TotalConfig,
        };
        let spec = RollSpec {
            expr: Expr::Const(ConstTerm { value: 1, label: None }),
            direction: Direction::HighWins,
            mode: Mode::Total(TotalConfig {
                difficulty: Some(0),
                tiers: vec![
                    Tier { margin_offset: 5, label: Some("a".into()), tier_value: Some(1) },
                    Tier { margin_offset: 5, label: Some("b".into()), tier_value: Some(2) },
                ],
            }),
        };
        assert!(matches!(
            validate_pre_roll(&spec),
            Err(RollError::DuplicateTierOffset(5))
        ));
        // Unique offsets pass.
        let mut ok = spec.clone();
        if let Mode::Total(cfg) = &mut ok.mode {
            cfg.tiers[1].margin_offset = 6;
        }
        assert!(validate_pre_roll(&ok).is_ok());
    }
```

(`validate_pre_roll` is private — the test lives in rolls.rs's own tests module, which
already tests it; follow the existing convention there.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml duplicate_tier_offsets`
Expected: FAIL (no variant / no check).

- [ ] **Step 3: Implement**

Add the variant to `RollError` with its Display arm (beside `ExpertiseTooLarge`, rolls.rs:195-205):

```rust
    /// Two ladder rungs share one `margin_offset` — `classify`'s
    /// max_by_key/min_by_key tie is caller-order-dependent, so which rung wins
    /// would be nondeterministic. Refused at construction so every downstream
    /// ladder is unambiguous (classify.rs's doc comment documents the tie).
    DuplicateTierOffset(i32),
```

```rust
            RollError::DuplicateTierOffset(o) => {
                write!(f, "duplicate tier margin offset {o}")
            }
```

Add beside `validate_pre_roll`:

```rust
/// Uniqueness guard over a classification ladder's `margin_offset`s. Notation
/// cannot author a non-empty ladder today (parser.rs emits `tiers: vec![]`),
/// so this arms the boundary for the tier-ladder syntax before it exists —
/// the guard predates the untrusted path by construction.
fn validate_tiers(tiers: &[crate::dice::spec::Tier]) -> Result<(), RollError> {
    let mut seen = std::collections::BTreeSet::new();
    for t in tiers {
        if !seen.insert(t.margin_offset) {
            return Err(RollError::DuplicateTierOffset(t.margin_offset));
        }
    }
    Ok(())
}
```

Wire into `validate_pre_roll` (after the per-group loop, alongside the existing
`Mode::SuccessCount` expertise check — restructure to a full match):

```rust
    match &spec.mode {
        Mode::Total(cfg) => validate_tiers(&cfg.tiers)?,
        Mode::SuccessCount(cfg) => {
            validate_tiers(&cfg.tiers)?;
            if cfg.expertise > MAX_EXPERTISE {
                return Err(RollError::ExpertiseTooLarge(cfg.expertise));
            }
        }
    }
```

- [ ] **Step 4: Run the rolls suite**

Run: `cargo test --manifest-path src/server/Cargo.toml chat::rolls`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/chat/rolls.rs
git commit -m "feat(dice): reject duplicate tier margin offsets at the roll wire boundary"
```

---

### Task 12: `labeled_consts` — missing-key test, stale comment, effective-sign display

**Files:**
- Modify: `src/server/src/dice/eval/sum.rs:39-61` (`collect_labeled_consts` + call site :20-21)
- Modify: `src/server/tests/chat_rolls.rs:356-359` (stale doc comment only)
- Test: `src/server/src/dice/outcome.rs` tests + `src/server/src/dice/eval/sum.rs` tests

**Interfaces:**
- Consumes: `Expr`/`BinOp`/`ConstTerm` (spec.rs:134-166), `RollOutcome` (outcome.rs:103-136).

- [ ] **Step 1: Write the failing tests**

In `outcome.rs` tests (the genuinely missing serde pin):

```rust
    #[test]
    fn roll_outcome_missing_defaulted_keys_deserializes() {
        // Pins `#[serde(default)]` on labeled_consts + symbol_counts against a
        // pre-M11d/pre-M13d stored RollOutcome shape (no such Rust-side test
        // existed; the chat_rolls back-compat test carries no RollOutcome).
        let j = serde_json::json!({
            "total": 7, "records": [], "successes": null, "pass": null,
            "margin": null, "tier_label": null, "tier_value": null,
            "crit_successes": 0, "crit_fails": 0,
            "positive_counter": 0, "negative_counter": 0
        });
        let out: super::RollOutcome = serde_json::from_value(j).unwrap();
        assert!(out.labeled_consts.is_empty());
        assert!(out.symbol_counts.is_empty());
    }
```

In `sum.rs` tests (beside `labeled_bare_constant_surfaces_in_labeled_consts` at :332):

```rust
    #[test]
    fn labeled_const_display_carries_effective_sign() {
        // Negation: "-3[dex]" displays -3.
        let spec = notation::parse("-3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, -3);
        // Subtraction (the common authoring shape): "1d20 - 3[dex]" → -3.
        let spec = notation::parse("1d20 - 3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, -3);
        // Double negation cancels: "1d20 - -3[dex]" → 3.
        let spec = notation::parse("1d20 - -3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, 3);
        // Multiplication keeps the literal (documented): "2 * 3[dex]" → 3.
        let spec = notation::parse("2 * 3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, 3);
    }
```

(If the notation grammar rejects any of these strings — e.g. unary minus on a labeled const —
construct the equivalent `Expr` tree directly with struct literals instead; the tree shapes
are `Neg(Const)`, `Bin{Sub, Dice, Const}`, `Bin{Sub, Dice, Neg(Const)}`, `Bin{Mul, Const, Const}`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml labeled_const_display roll_outcome_missing`
Expected: the serde test PASSES immediately (it pins existing behavior — keep it); the sign
test FAILS (value 3 where -3 expected).

- [ ] **Step 3: Implement the sign threading**

Replace `collect_labeled_consts` (sum.rs:39-61):

```rust
/// Collects every labeled `Const` term, in AST left-to-right order, for
/// chat-embed display (`RollOutcome::labeled_consts`), carrying the term's
/// EFFECTIVE additive sign: negation (`Neg`) and the right side of `Sub` flip
/// it, so `-3[dex]` and `1d20 - 3[dex]` both display -3. Multiplicative
/// context (`Mul`/`Div`) is NOT folded in — a `2 * 3[dex]` displays its
/// literal 3, mirroring how a `DieRecord`'s raw face is shown regardless of
/// the arithmetic around its group. Total mode only; SuccessCount ignores the
/// arithmetic entirely.
fn collect_labeled_consts(expr: &Expr, sign: i32, out: &mut Vec<ConstTerm>) {
    match expr {
        Expr::Const(c) => {
            if c.label.is_some() {
                out.push(ConstTerm {
                    // saturating_neg: i32::MIN has no i32 negation.
                    value: if sign < 0 { c.value.saturating_neg() } else { c.value },
                    label: c.label.clone(),
                });
            }
        }
        Expr::Dice(_) => {}
        Expr::Neg(inner) => collect_labeled_consts(inner, -sign, out),
        Expr::Bin { op: BinOp::Sub, lhs, rhs } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, -sign, out);
        }
        Expr::Bin { lhs, rhs, .. } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, sign, out);
        }
    }
}
```

Call site (sum.rs:21): `collect_labeled_consts(&spec.expr, 1, &mut labeled_consts);`
(`BinOp` needs importing into sum.rs if not already.)

- [ ] **Step 4: Fix the stale test comment**

In `tests/chat_rolls.rs:356-359`, the doc comment claims `content: []` while the fixture uses
a one-element text array, and implies RollOutcome coverage it doesn't have. Replace with:

```rust
/// (g) A stored pre-M11d-2 `MessageEngine` JSON (no roll segments) still
/// round-trips — the roll `Segment` variants are additive. RollOutcome
/// missing-key back-compat is pinned separately in `dice::outcome`'s
/// `roll_outcome_missing_defaulted_keys_deserializes`.
```

- [ ] **Step 5: Run the suite, then commit**

Run: `cargo test --manifest-path src/server/Cargo.toml`
Expected: PASS (also confirms the client-mirror Zod default is untouched — no client change).

```bash
git add src/server/src/dice/eval/sum.rs src/server/src/dice/outcome.rs src/server/tests/chat_rolls.rs
git commit -m "fix(dice): labeled consts carry effective additive sign; pin RollOutcome serde defaults"
```

---

### Task 13: Atomic restore swap

**Files:**
- Modify: `src/server/src/backup.rs:161-223` (`restore_backup`)
- Test: `src/server/src/backup.rs` tests

**Interfaces:**
- Consumes: existing `copy_dir_recursive`, `dir_is_empty_or_absent`, `file_absent`, `BackupError`.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn restore_leaves_no_partial_destination_and_no_staging_residue() {
        // After a successful force-restore over existing content: destination
        // matches the backup exactly and neither staging path
        // (assets.restore-tmp / assets.restore-old, db restore-tmp) survives.
        let tmp = tempfile::tempdir().unwrap();
        // ... build a backup dir via create_backup exactly as the existing
        //     restore tests do (backup.rs:342-385 shape) ...
        // ... seed DIFFERENT pre-existing destination content ...
        restore_backup(&backup_dir, &db_path, &assets_dir, true).await.unwrap();
        // Destination content equals backup content (reuse the existing
        // content assertions from the force-restore test).
        let parent = assets_dir.parent().unwrap();
        assert!(!parent.join("assets.restore-tmp").exists());
        assert!(!parent.join("assets.restore-old").exists());
        assert!(!db_path.with_extension("restore-tmp").exists());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml restore_leaves_no_partial`
Expected: FAIL (staging paths never created — assert on them proves the new mechanism once
implemented; before implementation the direct-copy path passes content checks but this test
should assert the staging protocol by checking a marker — if it passes vacuously, tighten by
asserting the implementation detail after Step 3 instead; the load-bearing regression
coverage is the existing force/content tests staying green).

- [ ] **Step 3: Implement**

Replace restore_backup's destination writes (:211-220) with a stage-then-swap:

```rust
    // Stage-then-swap: every fallible copy lands in a sibling staging path
    // first; the destination is only ever touched by rename, so a failure at
    // any point leaves it either fully pre-restore or fully post-restore
    // (worst case: the old assets dir parked at `assets.restore-old`, which
    // the next restore clears). `std::fs::rename` replaces an existing FILE
    // on all three target OSes but not a non-empty directory — hence the
    // two-step dir swap.
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let db_stage = db_path.with_extension("restore-tmp");
    tokio::fs::copy(&backup_db, &db_stage).await?;
    tokio::fs::rename(&db_stage, db_path).await?;

    let assets_name = assets_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "assets".to_string());
    let assets_parent = assets_dir.parent().map(Path::to_path_buf).unwrap_or_default();
    let assets_stage = assets_parent.join(format!("{assets_name}.restore-tmp"));
    let assets_old = assets_parent.join(format!("{assets_name}.restore-old"));
    if tokio::fs::metadata(&assets_stage).await.is_ok() {
        tokio::fs::remove_dir_all(&assets_stage).await?;
    }
    if tokio::fs::metadata(&assets_old).await.is_ok() {
        tokio::fs::remove_dir_all(&assets_old).await?;
    }
    let backup_assets = backup_dir.join("assets");
    copy_dir_recursive(&backup_assets, &assets_stage).await?;
    if tokio::fs::metadata(assets_dir).await.is_ok() {
        tokio::fs::rename(assets_dir, &assets_old).await?;
    }
    tokio::fs::rename(&assets_stage, assets_dir).await?;
    if tokio::fs::metadata(&assets_old).await.is_ok() {
        tokio::fs::remove_dir_all(&assets_old).await?;
    }

    Ok(())
```

Update the fn doc comment (:161-169): document the staging protocol and the
`assets.restore-old` recovery note. Delete the old copy/remove block it replaces.

- [ ] **Step 4: Run the backup suite**

Run: `cargo test --manifest-path src/server/Cargo.toml backup`
Expected: PASS (all 12 existing tests + the new one).

- [ ] **Step 5: Commit**

```bash
git add src/server/src/backup.rs
git commit -m "fix(server): restore_backup stages and swaps atomically"
```

---

### Task 14: Write-quiesce barrier + admin in-server backup route

**Files:**
- Modify: `src/server/src/http/mod.rs` (`AppState` field + route + tests), `src/server/src/main.rs`, `src/server/src/bin/test_server.rs`
- Modify: `src/server/src/http/assets.rs` (`upload` :159-217, `replace` :261-340)
- Modify: `src/server/src/http/routes.rs` (new handler), `src/server/src/config.rs` (backups path)
- Test: `src/server/src/http/mod.rs` tests

**Interfaces:**
- Produces: `AppState.write_barrier: Arc<tokio::sync::RwLock<()>>`; `POST /api/admin/backup`
  (admin-gated) → `Json<BackupManifest>`; `Config::backups_path() -> PathBuf`.
- Consumes: Task 13's restored `create_backup` (unchanged signature, backup.rs:106).

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn admin_backup_is_admin_gated_and_writes_a_manifest() {
    let state = initialized_state().await;
    // Point db + backups at a tempdir via the Config the state carries —
    // build state manually with a Config whose db is a real file (connect a
    // file-backed repo the way backup.rs tests do at :273-285).
    // ... state construction with tempdir-backed config ...
    let admin = { seed_admin(&state, "root").await; login_server(&state, "root").await };
    let user = { seed_user(&state, "pleb").await; login_server(&state, "pleb").await };

    user.post("/api/admin/backup").await.assert_status(axum::http::StatusCode::FORBIDDEN);

    let res = admin.post("/api/admin/backup").await;
    res.assert_status_ok();
    let manifest: serde_json::Value = res.json();
    assert!(manifest.get("db_bytes").and_then(|v| v.as_u64()).unwrap_or(0) > 0);
}

#[tokio::test]
async fn write_barrier_blocks_asset_writes_while_backup_holds_it() {
    let barrier = std::sync::Arc::new(tokio::sync::RwLock::<()>::new(()));
    let quiesce = barrier.write().await; // backup in progress
    let b2 = barrier.clone();
    let attempt = tokio::spawn(async move {
        let _w = b2.read().await; // the asset write's guard
        true
    });
    tokio::task::yield_now().await;
    assert!(!attempt.is_finished(), "asset write must wait behind the quiesce");
    drop(quiesce);
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1), attempt)
        .await
        .unwrap()
        .unwrap());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src/server/Cargo.toml admin_backup write_barrier`
Expected: FAIL (no route, no field).

- [ ] **Step 3: Implement**

`config.rs` — mirror `assets_path`'s shape (:142-151):

```rust
    /// Directory for in-server backups (`POST /api/admin/backup`): sibling of
    /// the DB file, `<db-parent>/backups`, unless `backups_dir` overrides it.
    pub fn backups_path(&self) -> std::path::PathBuf {
        match &self.backups_dir {
            Some(d) => std::path::PathBuf::from(d),
            None => std::path::Path::new(&self.db)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("backups"),
        }
    }
```

(Add `backups_dir: Option<String>` to `Config` + `Cli` + env layering following exactly how
`assets_dir` is declared and defaulted in the same file; `Default` impl gets `None`.)

`AppState`: add `pub write_barrier: Arc<tokio::sync::RwLock<()>>,` — initialize
`Arc::new(tokio::sync::RwLock::new(()))` at all three construction sites.

`routes.rs` — the handler (gate exactly as `POST /api/users` gates admins; reuse its check —
it compares `user.role` against `ServerRole::Admin`, grep `create_user_route` for the helper;
inline fallback shown):

```rust
/// In-server whole-server backup (admin only). Holds the write barrier in
/// write mode across the snapshot, so no asset commit+rename interleaves with
/// the `VACUUM INTO` + assets copy — the backup's DB metadata and file bytes
/// are mutually consistent. DB writers need no gating: VACUUM INTO is
/// transactionally consistent against a live writer by itself.
pub async fn admin_backup(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<crate::backup::BackupManifest>, AppError> {
    if user.role != crate::auth::role::ServerRole::Admin {
        return Err(AppError::Forbidden);
    }
    let _quiesce = state.write_barrier.write().await;
    let out = state
        .config
        .backups_path()
        .join(format!("backup-{}", now_millis()));
    let manifest = crate::backup::create_backup(
        std::path::Path::new(&state.config.db),
        &state.config.assets_path(),
        &out,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "in-server backup failed");
        AppError::Internal
    })?;
    Ok(Json(manifest))
}
```

Route registration in `http/mod.rs`: `.route("/api/admin/backup", post(routes::admin_backup))`.
Note: `create_backup` opens its own single-connection pool against the same SQLite file —
if the live pool's connection makes `VACUUM INTO` return busy, map it to `AppError::Internal`
(the admin retries); do not add a busy-loop.

`assets.rs` — in `upload` and `replace`, acquire the barrier in read mode around the fallible
write block (the `let outcome: Result<Asset, AppError> = async { ... }` at replace :293-332
and upload's equivalent):

```rust
    // Read-side of the backup quiesce barrier: an in-server backup takes the
    // write side across VACUUM + assets copy, so no row-commit/rename pair
    // can straddle the snapshot. Concurrent asset writes share the read side
    // freely — this serializes nothing between uploads.
    let _write_permit = state.write_barrier.read().await;
```

placed immediately before each `let outcome` block (held across DB commit + rename).

- [ ] **Step 4: Run the suite**

Run: `cargo test --manifest-path src/server/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/http/mod.rs src/server/src/http/routes.rs src/server/src/http/assets.rs src/server/src/config.rs src/server/src/main.rs src/server/src/bin/test_server.rs
git commit -m "feat(server): admin in-server backup with write-quiesce barrier on asset writes"
```

---

### Task 15: Gates, docs, skills

**Files:**
- Modify: `docs/TODO.md`, `docs/POST_WORK_FINDINGS.md`, `docs/PLAN.md`
- Modify: `.claude/skills/shadowcat-codebase-{realtime-sync,scene-rendering,dice,documents-permissions,server-ops,chat}/SKILL.md` (as applicable)

- [ ] **Step 1: Full gates**

```bash
cargo fmt --manifest-path src/server/Cargo.toml
cargo clippy --manifest-path src/server/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src/server/Cargo.toml
```

Expected: all clean/green. Fix anything that isn't before proceeding.

- [ ] **Step 2: TODO.md closures** — delete these entries (each now shipped): rate-limit
  `/api/login`; rate-limit `/api/invites/accept`; GC `world_invites`; validate
  `TokenEngine.x/y`; the four/six surviving `unwrap_or(100.0)`; `ScenePing` accepts any scene
  id; `handle_send_message` scope pin (the "Blocked on real-world need" bullet); both backup
  atomicity bullets; the `DieKind::Faces` item's remaining `(2)` (ReplaceDie); the Tier
  `margin_offset` item. Also fix the stale hex entry ("Actionable now — server-side hex-grid
  movement support"): the work is DONE (`grid_shape.rs` + parity battery,
  plan `2026-07-22-hex-grid-server-movement.md`) — delete the entry.

- [ ] **Step 3: POST_WORK_FINDINGS closures** — mark Resolved with one-line pointers: the
  `apply_command` `/engine` gate residual (Task 9); the M13d labeled_consts test gap (Task
  12); the M13d `-3[dex]` display gap (Task 12). Leave every other entry for its owning phase
  per the campaign spec's Stage-0 table.

- [ ] **Step 4: Skill updates (reviewed gate)** — update the affected
  `shadowcat-codebase-*` skills: `realtime-sync` (auth throttle + ScenePing guard seam),
  `scene-rendering` (the never-fork table's `unwrap_or` row: now REMOVED everywhere; absent
  scene is explicit at all six sites), `dice` (ReplaceDie gate + tier-ladder guard at
  `validate_pre_roll`), `documents-permissions` (apply_command now carries the engine gate),
  `server-ops` (in-server backup route + write barrier + backups_path), `chat` (send-message
  world pin). Dispatch `shadowcat-spec-reviewer` on the skill diffs (delivery: findings
  returned as the agent result) and fix anything it flags.

- [ ] **Step 5: PLAN.md** — add a Phase-A completion line under the campaign's entry
  (create the campaign entry referencing the spec if absent).

- [ ] **Step 6: Final review + merge + push** — per the Buddy-check directives: dispatch the
  `shadowcat-spec-reviewer` + `shadowcat-code-reviewer` pair over the whole branch diff
  (delivery: findings returned as agent results). Resolve findings, re-run gates, merge
  `--no-ff` to main, push, `gh run watch` (three-OS matrix must be green).

```bash
git add docs/TODO.md docs/POST_WORK_FINDINGS.md docs/PLAN.md .claude/skills
git commit -m "docs: Phase A close-out — TODO/findings closures, skill updates"
```

---

## Self-review record

- **Spec coverage:** A1→Tasks 1-2; A2→Task 3; A3→Task 4; A4→Tasks 5-6; A5→Task 7; A6→Task 8;
  A7→Tasks 13-14; A8→Task 9; A9→Tasks 10-11; A10→Task 12. Doc/skill sync→Task 15. No gaps.
- **Deviation from spec, flagged:** A7's quiesce ships as an admin in-server backup route
  (design decision 4) because CLI backup runs cross-process where an in-process barrier
  cannot reach — new API surface (`POST /api/admin/backup`), justified by Phase-4 backup
  scheduling needing it regardless. Reviewer should confirm the user saw this.
- **Type consistency:** `AuthThrottle.check(&str, i64, usize)` used identically in Tasks 1-2;
  `region_field -> Option<RegionField>` consumed by both callers in Task 5 only;
  `scene_ping_permitted` signature matches its single call site; `collect_labeled_consts`
  arity change is confined to sum.rs.
