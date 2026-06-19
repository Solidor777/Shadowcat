# M7a — Server Surface Implementation Plan

## Buddy-check directives

This branch touches an **auth-adjacent read surface** (world-list visibility +
`ui_state` on the users table + a public pre-init config probe), flagged
high-risk in spec §13. Final review is a **buddy-check** (two independent blind
reviewers + structured debate via the `buddy-checking` skill), chosen by the
human at execution handoff — not the single-reviewer default. Focus the
reviewers on: cross-user world leakage, the opaque-blob validation boundary, and
the pre-init reachability of `/api/config`.


> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the additive server endpoints the M7 UI entry flow needs — list a
user's worlds, expose whether the server is initialized, and persist a per-user
opaque UI-state blob — without disturbing the existing static auth flow.

**Architecture:** Three additive HTTP read/write surfaces on the existing axum
router (`src/server/src/http/`), backed by new inherent methods on
`SqliteRepository` and one migration adding a nullable `ui_state` column to
`users`. New client-consumed response DTOs are ts-rs-exported to
`src/types/generated/` (the existing type pipeline). The `embed.rs` seam flip and
`init_gate` rework are explicitly **deferred to M7c** (they require the Svelte
bundle to exist; flipping now would break the running app's setup redirect).

**Tech Stack:** Rust, axum 0.8, sqlx (SQLite), tower-sessions, ts-rs, async-trait,
`axum_test` for handler tests.

## Global Constraints

- All routes are under `/api/*` (e.g. `/api/me`, `/api/worlds`). New routes:
  `GET /api/worlds`, `GET /api/config`, `GET|PUT /api/me/ui-state`.
- Cross-platform: portable SQL only; no OS-specific code. CI matrix
  (ubuntu/macos/windows) must stay green.
- ts-rs DTOs consumed by the client derive `TS` with
  `#[ts(export, export_to = "../../types/generated/")]`; run `cargo test` to
  regenerate, and commit the generated `.ts` alongside the Rust change (CI
  enforces sync).
- Opaque-blob invariant (ARCHITECTURE §"Validation at boundaries"): the server
  validates `ui_state` is a JSON **object** under a size cap and never interprets
  its contents.
- Authorization-correctness invariant: `GET /api/worlds` must return only worlds
  the caller may access — a member sees their worlds, a server admin sees all,
  and no user sees another user's worlds.
- TDD: write the failing test first, watch it fail, implement minimally, watch it
  pass, commit. Tests for repository methods live in the `sqlite.rs` `#[cfg(test)]`
  module; route tests live in the `http/mod.rs` `tests` module (alongside the
  existing M3/M5 route tests).
- Run a single named test with `cargo test <name>` from the repo root (filters by
  name across the workspace). The tested code + ts-rs `TS` types live in the
  **lib** crate (`src/server/src/lib.rs`), so regenerate bindings and run the full
  suite with `cargo test --lib` (bindings) / `cargo test -p shadowcat` (lib + bin
  + integration). NOTE: `--bin shadowcat` runs zero of these tests.

---

### Task 1: `ui_state` column + repository accessors

**Files:**
- Create: `src/server/migrations/0004_user_ui_state.sql`
- Modify: `src/server/src/data/sqlite.rs` (add `get_ui_state` / `set_ui_state`
  inherent methods near `create_user`, ~line 214; add tests in the `tests` module)

**Interfaces:**
- Produces:
  - `SqliteRepository::get_ui_state(&self, user: Uuid) -> Result<Option<String>, DataError>`
    — the stored JSON string, or `None` when unset.
  - `SqliteRepository::set_ui_state(&self, user: Uuid, json: &str) -> Result<(), DataError>`
    — replace; `DataError::NotFound` if the user row is absent.

- [ ] **Step 1: Write the migration**

`src/server/migrations/0004_user_ui_state.sql`:

```sql
-- Per-user opaque UI session state (active world, active tab, locale, ...).
-- The server stores it verbatim and validates only object-shape + size cap;
-- the client owns the structure. NULL until the first PUT.
ALTER TABLE users ADD COLUMN ui_state TEXT;
```

- [ ] **Step 2: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/data/sqlite.rs`:

```rust
#[tokio::test]
async fn ui_state_round_trips_and_defaults_to_none() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let user = repo
        .create_user("u", Some("hash"), ServerRole::User, 0)
        .await
        .unwrap();

    // Unset → None.
    assert_eq!(repo.get_ui_state(user).await.unwrap(), None);

    // Set then read back verbatim.
    repo.set_ui_state(user, r#"{"global":{"locale":"en"}}"#)
        .await
        .unwrap();
    assert_eq!(
        repo.get_ui_state(user).await.unwrap().as_deref(),
        Some(r#"{"global":{"locale":"en"}}"#)
    );

    // Replace (not merge).
    repo.set_ui_state(user, r#"{"global":{"locale":"fr"}}"#)
        .await
        .unwrap();
    assert_eq!(
        repo.get_ui_state(user).await.unwrap().as_deref(),
        Some(r#"{"global":{"locale":"fr"}}"#)
    );

    // Unknown user → NotFound.
    let ghost = uuid::Uuid::from_u128(1);
    assert!(matches!(
        repo.set_ui_state(ghost, "{}").await,
        Err(DataError::NotFound)
    ));
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test ui_state_round_trips_and_defaults_to_none`
Expected: FAIL — `no method named get_ui_state`.

- [ ] **Step 4: Implement the accessors**

Add to `impl SqliteRepository` in `src/server/src/data/sqlite.rs` (after
`create_user`):

```rust
/// The user's stored opaque UI-state JSON string, or `None` when unset.
pub async fn get_ui_state(&self, user: Uuid) -> Result<Option<String>, DataError> {
    let row = sqlx::query("SELECT ui_state FROM users WHERE id = ?")
        .bind(user.to_string())
        .fetch_optional(&self.pool)
        .await?;
    Ok(row.and_then(|r| r.get::<Option<String>, _>("ui_state")))
}

/// Replace the user's opaque UI-state JSON. `NotFound` if the user is absent.
/// The string is stored verbatim; shape/size are validated at the HTTP boundary.
pub async fn set_ui_state(&self, user: Uuid, json: &str) -> Result<(), DataError> {
    let res = sqlx::query("UPDATE users SET ui_state = ? WHERE id = ?")
        .bind(json)
        .bind(user.to_string())
        .execute(&self.pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DataError::NotFound);
    }
    Ok(())
}
```

- [ ] **Step 5: Run it to verify it passes**

Run: `cargo test ui_state_round_trips_and_defaults_to_none`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/migrations/0004_user_ui_state.sql src/server/src/data/sqlite.rs
git commit -m "feat(server): ui_state column + get/set_ui_state repo accessors"
```

---

### Task 2: `GET|PUT /api/me/ui-state` endpoints

**Files:**
- Modify: `src/server/src/http/routes.rs` (add handlers + size cap near the top of
  the worlds/documents section)
- Modify: `src/server/src/http/mod.rs` (register the route; add a test)

**Interfaces:**
- Consumes: `SqliteRepository::get_ui_state` / `set_ui_state` (Task 1);
  `AuthUser` extractor (existing, `crate::auth::session::AuthUser`); `AppError`.
- Produces:
  - `routes::get_ui_state(user: AuthUser, State<AppState>) -> Result<Json<serde_json::Value>, AppError>`
    — the stored object, or `{}` when unset.
  - `routes::put_ui_state(user: AuthUser, State<AppState>, Json<serde_json::Value>) -> Result<StatusCode, AppError>`
    — `204` on success; `422` on non-object or over cap.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/server/src/http/mod.rs` (uses the existing
`initialized_state`, `seed_user`, `login_server` helpers):

```rust
#[tokio::test]
async fn ui_state_get_put_round_trip_and_validation() {
    let state = initialized_state().await;
    seed_user(&state, "u").await;
    let u = login_server(&state, "u").await;

    // Unauthenticated PUT/GET are rejected.
    let anon = axum_test::TestServer::builder()
        .save_cookies()
        .build(router(state.clone()).await)
        .unwrap();
    anon.get("/api/me/ui-state")
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Default is an empty object.
    let got: serde_json::Value = u.get("/api/me/ui-state").await.json();
    assert_eq!(got, serde_json::json!({}));

    // Store an object, read it back.
    u.put("/api/me/ui-state")
        .json(&serde_json::json!({ "global": { "locale": "en" } }))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: serde_json::Value = u.get("/api/me/ui-state").await.json();
    assert_eq!(got["global"]["locale"], "en");

    // A non-object body is rejected.
    u.put("/api/me/ui-state")
        .json(&serde_json::json!([1, 2, 3]))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // An over-cap body is rejected.
    let big = "x".repeat(70 * 1024);
    u.put("/api/me/ui-state")
        .json(&serde_json::json!({ "blob": big }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test ui_state_get_put_round_trip_and_validation`
Expected: FAIL — route returns 404 / handler not found.

- [ ] **Step 3: Implement the handlers**

Add to `src/server/src/http/routes.rs` (the `AuthUser` import already exists):

```rust
/// Upper bound on a stored UI-state blob. It is read/written whole per user and
/// is small UI session state; far above any realistic payload.
const MAX_UI_STATE_BYTES: usize = 64 * 1024;

/// The caller's opaque UI-state object, or `{}` when unset.
pub async fn get_ui_state(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let val = match state.repo.get_ui_state(user.id).await? {
        // Stored only after passing the object-shape check below, so a parse
        // failure here is a server-side corruption, not client-actionable.
        Some(s) => serde_json::from_str(&s).map_err(|_| AppError::Internal)?,
        None => serde_json::json!({}),
    };
    Ok(Json(val))
}

/// Replace the caller's UI-state. Validates object-shape + size only; the body
/// is otherwise opaque (the client owns its structure).
pub async fn put_ui_state(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    if !body.is_object() {
        return Err(AppError::Unprocessable("ui_state must be a JSON object".into()));
    }
    let s = serde_json::to_string(&body).map_err(|_| AppError::Internal)?;
    if s.len() > MAX_UI_STATE_BYTES {
        return Err(AppError::Unprocessable(format!(
            "ui_state too large (max {MAX_UI_STATE_BYTES} bytes)"
        )));
    }
    state.repo.set_ui_state(user.id, &s).await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Register the route**

In `src/server/src/http/mod.rs` `router()`, add after the `/api/me` route:

```rust
        .route(
            "/api/me/ui-state",
            get(routes::get_ui_state).put(routes::put_ui_state),
        )
```

- [ ] **Step 5: Run it to verify it passes**

Run: `cargo test ui_state_get_put_round_trip_and_validation`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "feat(server): GET/PUT /api/me/ui-state (opaque object, size-capped)"
```

---

### Task 3: `GET /api/config` (public initialized probe)

**Files:**
- Modify: `src/server/src/http/routes.rs` (add `ServerConfig` DTO + `config`
  handler)
- Modify: `src/server/src/http/mod.rs` (register the route; add a test)
- Modify: `src/server/src/http/middleware.rs` (allow `/api/config` pre-init)
- Generated: `src/types/generated/ServerConfig.ts` (via `cargo test`)

**Interfaces:**
- Produces:
  - `routes::ServerConfig { initialized: bool }` — ts-rs-exported.
  - `routes::config(State<AppState>) -> Json<ServerConfig>` — public, no auth.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/server/src/http/mod.rs`:

```rust
#[tokio::test]
async fn config_reports_initialized_state_and_is_public_pre_init() {
    // Uninitialized: reachable (not redirected to setup) and reports false.
    let fresh = fresh_server().await;
    let res = fresh.get("/api/config").await;
    res.assert_status_ok();
    assert_eq!(res.json::<serde_json::Value>()["initialized"], false);

    // Initialized: reports true.
    let server =
        axum_test::TestServer::new(router(initialized_state().await).await).unwrap();
    let res = server.get("/api/config").await;
    res.assert_status_ok();
    assert_eq!(res.json::<serde_json::Value>()["initialized"], true);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test config_reports_initialized_state_and_is_public_pre_init`
Expected: FAIL — pre-init request is redirected (303) by `init_gate`, and the
route does not exist.

- [ ] **Step 3: Implement the DTO + handler**

Add to `src/server/src/http/routes.rs` (imports `ts_rs::TS` at the top:
`use ts_rs::TS;`):

```rust
/// Public server bootstrap info for the SPA's first-load routing (setup vs
/// login). Exposes nothing beyond the `initialized` bit the setup-409 already
/// reveals.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ServerConfig {
    pub initialized: bool,
}

/// Whether a first admin exists. Unauthenticated; reachable before init.
pub async fn config(State(state): State<AppState>) -> Json<ServerConfig> {
    Json(ServerConfig {
        initialized: state.initialized.load(Ordering::Relaxed),
    })
}
```

(`Ordering` is already imported at the top of `routes.rs`.)

- [ ] **Step 4: Allow it pre-init + register the route**

In `src/server/src/http/middleware.rs`, add `/api/config` to the `init_gate`
allowlist:

```rust
    let allowed = matches!(
        path,
        "/api/setup" | "/api/config" | "/setup.html" | "/auth.js" | "/styles.css" | "/health"
    );
```

In `src/server/src/http/mod.rs` `router()`, add after the `/health` route:

```rust
        .route("/api/config", get(routes::config))
```

- [ ] **Step 5: Run it to verify it passes (regenerates the TS type)**

Run: `cargo test config_reports_initialized_state_and_is_public_pre_init`
Then: `cargo test` (runs the ts-rs `export_bindings_*` tests, regenerating
`src/types/generated/ServerConfig.ts`)
Expected: PASS; `src/types/generated/ServerConfig.ts` now exists.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs \
        src/server/src/http/middleware.rs src/types/generated/ServerConfig.ts
git commit -m "feat(server): public GET /api/config initialized probe"
```

---

### Task 4: `worlds_for_user` repository query

**Files:**
- Modify: `src/server/src/data/sqlite.rs` (add `worlds_for_user`; add tests)

**Interfaces:**
- Consumes: `World`, `WorldRole` (`crate::data::document`), `ServerRole`.
- Produces:
  - `SqliteRepository::worlds_for_user(&self, user: Uuid, server_role: ServerRole) -> Result<Vec<(World, WorldRole)>, DataError>`
    — worlds the user may access with their effective role. Admins resolve to GM
    on **every** world (mirrors `permission_context`); members get their joined
    role. Ordered by world name.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/server/src/data/sqlite.rs`:

```rust
#[tokio::test]
async fn worlds_for_user_scopes_to_membership_and_admin_sees_all() {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let a = repo.create_user("a", Some("h"), ServerRole::User, 0).await.unwrap();
    let b = repo.create_user("b", Some("h"), ServerRole::User, 0).await.unwrap();
    let admin = repo.create_user("ad", Some("h"), ServerRole::Admin, 0).await.unwrap();

    // a GMs world1; b GMs world2 (each creator seated as GM).
    let w1 = repo.create_world_owned("world1", a, 0).await.unwrap();
    let w2 = repo.create_world_owned("world2", b, 0).await.unwrap();
    // a is added to world2 as a player.
    repo.add_member(w2.id, a, WorldRole::Player).await.unwrap();

    // a sees only their two worlds, with the right roles; never b-only state.
    let mut a_worlds = repo.worlds_for_user(a, ServerRole::User).await.unwrap();
    a_worlds.sort_by(|x, y| x.0.name.cmp(&y.0.name));
    assert_eq!(a_worlds.len(), 2);
    assert_eq!((a_worlds[0].0.id, a_worlds[0].1), (w1.id, WorldRole::Gm));
    assert_eq!((a_worlds[1].0.id, a_worlds[1].1), (w2.id, WorldRole::Player));

    // b sees only world2.
    let b_worlds = repo.worlds_for_user(b, ServerRole::User).await.unwrap();
    assert_eq!(b_worlds.len(), 1);
    assert_eq!(b_worlds[0].0.id, w2.id);

    // A server admin sees every world as GM.
    let admin_worlds = repo.worlds_for_user(admin, ServerRole::Admin).await.unwrap();
    assert_eq!(admin_worlds.len(), 2);
    assert!(admin_worlds.iter().all(|(_, r)| *r == WorldRole::Gm));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test worlds_for_user_scopes_to_membership_and_admin_sees_all`
Expected: FAIL — `no method named worlds_for_user`.

- [ ] **Step 3: Implement the query**

Add to `impl SqliteRepository` in `src/server/src/data/sqlite.rs`:

```rust
/// Worlds the user may access, with their effective role. A server admin is GM
/// on every world (mirrors `permission_context`); otherwise the user's joined
/// `world_members.role`. Ordered by world name.
pub async fn worlds_for_user(
    &self,
    user: Uuid,
    server_role: ServerRole,
) -> Result<Vec<(World, WorldRole)>, DataError> {
    let rows = if server_role == ServerRole::Admin {
        sqlx::query(
            "SELECT id, name, seq, created_at, updated_at, NULL AS role \
             FROM worlds ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?
    } else {
        sqlx::query(
            "SELECT w.id, w.name, w.seq, w.created_at, w.updated_at, m.role AS role \
             FROM worlds w \
             JOIN world_members m ON m.world_id = w.id \
             WHERE m.user_id = ? ORDER BY w.name",
        )
        .bind(user.to_string())
        .fetch_all(&self.pool)
        .await?
    };

    rows.into_iter()
        .map(|r| {
            let world = World {
                id: Uuid::parse_str(r.get::<String, _>("id").as_str())
                    .map_err(|e| DataError::OpFailed(e.to_string()))?,
                name: r.get("name"),
                seq: r.get("seq"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            };
            // Admin rows carry NULL role → GM; member rows decode their stored role.
            let role = match r.get::<Option<String>, _>("role") {
                Some(s) => serde_json::from_value(serde_json::Value::String(s))?,
                None => WorldRole::Gm,
            };
            Ok((world, role))
        })
        .collect()
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test worlds_for_user_scopes_to_membership_and_admin_sees_all`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/server/src/data/sqlite.rs
git commit -m "feat(server): worlds_for_user query (membership-scoped, admin-all)"
```

---

### Task 5: `GET /api/worlds` endpoint

**Files:**
- Modify: `src/server/src/http/routes.rs` (add `WorldEntry` DTO + `list_worlds`)
- Modify: `src/server/src/http/mod.rs` (add `.get(...)` to the `/api/worlds`
  route; add a test)
- Generated: `src/types/generated/WorldEntry.ts` (via `cargo test`)

**Interfaces:**
- Consumes: `SqliteRepository::worlds_for_user` (Task 4); `AuthUser`; `WorldRole`.
- Produces:
  - `routes::WorldEntry { id: Uuid, name: String, role: WorldRole }` —
    ts-rs-exported.
  - `routes::list_worlds(user: AuthUser, State<AppState>) -> Result<Json<Vec<WorldEntry>>, AppError>`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/server/src/http/mod.rs`:

```rust
#[tokio::test]
async fn list_worlds_returns_only_callers_worlds() {
    let state = initialized_state().await;
    seed_user(&state, "a").await;
    seed_user(&state, "b").await;
    let a = login_server(&state, "a").await;
    let b = login_server(&state, "b").await;

    // a creates world1 (GM); b creates world2 (GM).
    a.post("/api/worlds")
        .json(&serde_json::json!({ "name": "world1" }))
        .await
        .assert_status_ok();
    b.post("/api/worlds")
        .json(&serde_json::json!({ "name": "world2" }))
        .await
        .assert_status_ok();

    // a sees exactly world1, as gm.
    let worlds: serde_json::Value = a.get("/api/worlds").await.json();
    let arr = worlds.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "world1");
    assert_eq!(arr[0]["role"], "gm");
    assert!(arr[0]["id"].is_string());

    // a never sees b's world.
    assert!(!worlds.to_string().contains("world2"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test list_worlds_returns_only_callers_worlds`
Expected: FAIL — `GET /api/worlds` 404 (only POST registered).

- [ ] **Step 3: Implement the DTO + handler**

Add to `src/server/src/http/routes.rs`:

```rust
/// A world the caller can access, with their effective role. The client's
/// world-select list item.
#[derive(Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct WorldEntry {
    pub id: Uuid,
    pub name: String,
    pub role: WorldRole,
}

/// Worlds the authenticated caller may access.
pub async fn list_worlds(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<WorldEntry>>, AppError> {
    let worlds = state.repo.worlds_for_user(user.id, user.role).await?;
    Ok(Json(
        worlds
            .into_iter()
            .map(|(w, role)| WorldEntry {
                id: w.id,
                name: w.name,
                role,
            })
            .collect(),
    ))
}
```

(`WorldRole` is already imported in `routes.rs`; `Uuid` and `Serialize` too.)

- [ ] **Step 4: Add GET to the route**

In `src/server/src/http/mod.rs`, change the worlds route:

```rust
        .route(
            "/api/worlds",
            post(routes::create_world).get(routes::list_worlds),
        )
```

- [ ] **Step 5: Run it to verify it passes (regenerates the TS type)**

Run: `cargo test list_worlds_returns_only_callers_worlds`
Then: `cargo test` (regenerates `src/types/generated/WorldEntry.ts`)
Expected: PASS; `WorldEntry.ts` exists.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs \
        src/types/generated/WorldEntry.ts
git commit -m "feat(server): GET /api/worlds (caller-scoped world list)"
```

---

### Task 6: Full-suite green + docs sync

**Files:**
- Modify: `docs/design/ARCHITECTURE.md` (note the three new endpoints under the
  HTTP surface description, if such a list exists; otherwise skip)
- Verify: no other changes

- [ ] **Step 1: Run the server crate's whole suite**

Run: `cargo test -p shadowcat`
Expected: PASS — all existing M3/M5 route tests plus the four new tests green
(lib + bin + integration targets).

- [ ] **Step 2: Confirm generated types are committed and in sync**

Run: `git status --porcelain src/types/generated/`
Expected: empty (ServerConfig.ts and WorldEntry.ts already committed in Tasks 3/5;
nothing uncommitted).

- [ ] **Step 3: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Update ARCHITECTURE.md if it enumerates the HTTP surface**

If `docs/design/ARCHITECTURE.md` lists HTTP endpoints, add: `GET /api/config`
(public init probe), `GET /api/worlds` (caller-scoped world list), and
`GET|PUT /api/me/ui-state` (per-user opaque UI state). If no such list exists,
make no change (do not invent a section).

- [ ] **Step 5: Commit (if ARCHITECTURE.md changed)**

```bash
git add docs/design/ARCHITECTURE.md
git commit -m "docs(arch): note M7a HTTP endpoints"
```

---

## Self-Review

**Spec coverage (spec §9 M7a items):**
- `GET /api/worlds` → Tasks 4 (query) + 5 (endpoint). ✓
- `GET /api/config` → Task 3. ✓
- `GET|PUT /api/me/ui-state` + migration → Tasks 1 (migration + repo) + 2 (endpoints). ✓
- `embed.rs` seam flip + static retirement + build ordering → **deliberately
  deferred to M7c** (requires the Svelte bundle; flipping before it exists breaks
  the setup redirect and serves an empty `dist/`). Recorded below; the spec's §13
  M7a bullet is updated to match.

**Placeholder scan:** No TBD/TODO; every code and test block is complete and
runnable. ✓

**Type consistency:** `get_ui_state`/`set_ui_state`, `worlds_for_user`,
`ServerConfig`, `WorldEntry`, `list_worlds`, `config`, `get_ui_state`/`put_ui_state`
names are used identically across tasks. `WorldEntry.role` serializes via the
existing `WorldRole` serde (lowercase `"gm"`/`"player"`), matching the Task 5 test
assertion `role == "gm"`. ✓

## Scope refinement discovered during planning

The spec assigned the `embed.rs` seam flip + static retirement to M7a. Reading
`init_gate` (redirects all non-allowlisted paths to `/setup.html` while
uninitialized) and `embed.rs` (embeds `static/`) shows the flip cannot ship
working software without the Svelte bundle that M7c produces — it would serve an
empty `dist/` and break the setup redirect. The flip, the `init_gate` rework
(serve the SPA for the setup view instead of redirecting), and static retirement
therefore move to **M7c**. M7a stays purely additive and independently shippable.
Update spec §9.4 / §13 M7a accordingly at execution start.
