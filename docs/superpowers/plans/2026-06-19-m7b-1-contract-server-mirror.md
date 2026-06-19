# M7b-1 — Contract Schema + Server Mirror Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this project executes plans with
> the **mainline-plan-execution** skill (inline, per-task spec-compliance check +
> a single final branch review) — NOT subagent-driven-development or
> executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

## Buddy-check directives

This branch adds a **new GM-only world-write surface** plus a **`Welcome` frame
change**, flagged a buddy-check candidate in spec §7. Final review is a
**buddy-check** (two independent blind reviewers + debate). Focus the reviewers
on: the PUT validation completeness (dangling-requires, duplicate-singleton,
malformed-contract, count bound), GM-only gating, and the `Welcome` field
addition not breaking existing handshakes.

**Goal:** Mirror UI contract declarations server-side — a shared ts-rs schema,
per-world GM-published storage, validation, and `Welcome` broadcast — mirroring
the existing capability-requirements pattern exactly.

**Architecture:** New `ContractDeclaration` types in `data/document.rs`
(ts-rs-exported, shared Rust↔TS). Stored as JSON under a per-world key in the
existing `settings` table (no migration), via parallel `SqliteRepository`
accessors. GM-only `GET/PUT /api/worlds/{id}/contracts` with fail-closed
validation. A new `contract_declarations` field on `ServerMsg::Welcome`, loaded
once per connection in the egress task.

**Tech Stack:** Rust, axum 0.8, sqlx (SQLite), ts-rs, `axum_test`.

## Global Constraints

- Mirrors the capability-requirements pattern throughout: storage
  (`set_setting`/`get_setting` + a `world_contracts:{world}` key, like
  `world_caps_req_key`), endpoint shape (`require_gm`-gated GET/PUT like
  `get/set_world_capability_requirements`), validation rigor (fail-closed 422
  with specific messages, like `set_world_capability_requirements`), and the
  `Welcome` broadcast (loaded once per connection in `conn.rs`, like
  `capability_requirements`).
- The server stores/validates/distributes **declaration strings only** — never
  components, never module code (module-free invariant).
- Routes under `/api/*`. Client-consumed DTOs derive `TS` with
  `#[ts(export, export_to = "../../types/generated/")]`; run `cargo test --lib`
  to regenerate and commit the generated `.ts` (CI enforces sync). NOTE: the
  tested code + `TS` types live in the **lib** crate; `--bin shadowcat` runs zero
  of these tests. Full suite: `cargo test -p shadowcat`.
- TDD: failing test first, watch it fail, minimal impl, watch it pass, commit.
  Repo tests in the `sqlite.rs` `#[cfg(test)]` module; handler tests in the
  `http/mod.rs` `tests` module; protocol tests in the `ws/protocol.rs` `tests`
  module.

---

### Task 1: `ContractDeclaration` types + repository storage

**Files:**
- Modify: `src/server/src/data/document.rs` (add the three types near
  `CapabilityRequirement`, ~line 79)
- Modify: `src/server/src/data/repository.rs` (add `world_contract_declarations`
  trait getter near `world_cap_requirements`, ~line 54)
- Modify: `src/server/src/data/sqlite.rs` (add the key helper, the inherent
  setter near `set_world_cap_requirements` ~line 383, the trait-impl getter near
  `world_cap_requirements` ~line 920; add a test)
- Generated: `src/types/generated/{Cardinality,ContractProvide,ContractDeclaration}.ts`

**Interfaces:**
- Produces:
  - `data::document::Cardinality` (`Singleton` | `Multi`, serde snake_case),
    `ContractProvide { contract: String, cardinality: Cardinality }`,
    `ContractDeclaration { module_id: String, version: String, provides:
    Vec<ContractProvide>, requires: Vec<String> }` — all ts-rs-exported.
  - `SqliteRepository::set_world_contract_declarations(&self, world: Uuid, decls: &[ContractDeclaration]) -> Result<(), DataError>` (inherent).
  - `Repository::world_contract_declarations(&self, world: Uuid) -> Result<Vec<ContractDeclaration>, DataError>` (trait + impl; empty Vec when unset).

- [ ] **Step 1: Write the types**

In `src/server/src/data/document.rs`, after the `CapabilityRequirement` struct
(~line 79):

```rust
/// Cardinality of a UI surface contract: one provider or many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    Singleton,
    Multi,
}

/// A UI surface contract a module provides, with its cardinality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractProvide {
    pub contract: String,
    pub cardinality: Cardinality,
}

/// A module's UI contract declaration: what surface contracts it provides and
/// which it requires an active provider for. Pure data — the server validates
/// and distributes these strings; it never holds components or runs module code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct ContractDeclaration {
    pub module_id: String,
    pub version: String,
    #[serde(default)]
    pub provides: Vec<ContractProvide>,
    #[serde(default)]
    pub requires: Vec<String>,
}
```

- [ ] **Step 2: Write the failing repository test**

Add to the `#[cfg(test)] mod tests` block in `src/server/src/data/sqlite.rs`:

```rust
#[tokio::test]
async fn contract_declarations_round_trip_and_default_empty() {
    use crate::data::document::{Cardinality, ContractDeclaration, ContractProvide};
    let repo = repo().await;
    let world = repo.create_world("w", 0).await.unwrap();

    // Unset → empty.
    assert!(repo
        .world_contract_declarations(world.id)
        .await
        .unwrap()
        .is_empty());

    let decls = vec![ContractDeclaration {
        module_id: "core-ui".into(),
        version: "0.1.0".into(),
        provides: vec![ContractProvide {
            contract: "shadowcat.surface:sidebar".into(),
            cardinality: Cardinality::Singleton,
        }],
        requires: vec![],
    }];
    repo.set_world_contract_declarations(world.id, &decls)
        .await
        .unwrap();

    let got = repo.world_contract_declarations(world.id).await.unwrap();
    assert_eq!(got, decls);
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test contract_declarations_round_trip_and_default_empty`
Expected: FAIL — `no method named set_world_contract_declarations`.

- [ ] **Step 4: Add the key helper + setter**

In `src/server/src/data/sqlite.rs`, add the key helper next to
`world_caps_req_key` (~line 1044):

```rust
/// Settings key holding a world's UI contract declarations (JSON).
fn world_contracts_key(world: Uuid) -> String {
    format!("world_contracts:{world}")
}
```

Add the inherent setter after `set_world_cap_requirements` (~line 383):

```rust
/// Replace a world's UI contract declarations (stored as JSON in settings).
pub async fn set_world_contract_declarations(
    &self,
    world: Uuid,
    decls: &[ContractDeclaration],
) -> Result<(), DataError> {
    let json = serde_json::to_string(decls)?;
    self.set_setting(&world_contracts_key(world), &json).await
}
```

Add `ContractDeclaration` to the `crate::data::document` import at the top of
`sqlite.rs` (the existing `use crate::data::document::{ ... }` list).

- [ ] **Step 5: Add the trait getter**

In `src/server/src/data/repository.rs`, add to the `Repository` trait (after
`world_cap_requirements`, ~line 57), and add `ContractDeclaration` to the
`crate::data::document` import:

```rust
/// A world's UI contract declarations (GM-published). Empty when unset.
async fn world_contract_declarations(
    &self,
    world: Uuid,
) -> Result<Vec<ContractDeclaration>, DataError>;
```

In `src/server/src/data/sqlite.rs`, add to the `impl Repository for
SqliteRepository` block (after `world_cap_requirements`, ~line 920):

```rust
async fn world_contract_declarations(
    &self,
    world: Uuid,
) -> Result<Vec<ContractDeclaration>, DataError> {
    match self.get_setting(&world_contracts_key(world)).await? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}
```

- [ ] **Step 6: Run it to verify it passes + regenerate bindings**

Run: `cargo test contract_declarations_round_trip_and_default_empty`
Then: `cargo test --lib` (regenerates `Cardinality.ts`, `ContractProvide.ts`,
`ContractDeclaration.ts`)
Expected: PASS; the three `.ts` files exist under `src/types/generated/`.

- [ ] **Step 7: Commit**

```bash
git add src/server/src/data/document.rs src/server/src/data/repository.rs \
        src/server/src/data/sqlite.rs src/types/generated/Cardinality.ts \
        src/types/generated/ContractProvide.ts src/types/generated/ContractDeclaration.ts
git commit -m "feat(server): ContractDeclaration types + per-world settings storage"
```

---

### Task 2: GM-only `GET/PUT /api/worlds/{id}/contracts` + validation

**Files:**
- Modify: `src/server/src/http/routes.rs` (add `MAX_CONTRACT_DECLARATIONS`,
  `validate_contract_token`, `validate_contract_declarations`, the two handlers —
  near the capability-requirements handlers, ~line 526+)
- Modify: `src/server/src/http/mod.rs` (register the route; add tests)

**Interfaces:**
- Consumes: `ContractDeclaration`, `ContractProvide`, `Cardinality` (Task 1);
  `Repository::world_contract_declarations` /
  `SqliteRepository::set_world_contract_declarations`; `require_gm`, `AuthUser`,
  `AppError`.
- Produces:
  - `routes::get_world_contract_declarations(user, State, Path<Uuid>) -> Result<Json<Vec<ContractDeclaration>>, AppError>`.
  - `routes::set_world_contract_declarations(user, State, Path<Uuid>, Json<Vec<ContractDeclaration>>) -> Result<StatusCode, AppError>`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/server/src/http/mod.rs`:

```rust
#[tokio::test]
async fn contract_declarations_gm_crud_and_validation() {
    let state = initialized_state().await;
    seed_user(&state, "gm").await;
    let player_id = seed_user(&state, "pl").await;
    let gm = login_server(&state, "gm").await;
    let pl = login_server(&state, "pl").await;

    let world: serde_json::Value = gm
        .post("/api/worlds")
        .json(&serde_json::json!({ "name": "W" }))
        .await
        .json();
    let world_id = world["id"].as_str().unwrap().to_string();
    gm.post(&format!("/api/worlds/{world_id}/members"))
        .json(&serde_json::json!({ "user": player_id, "role": "player" }))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let valid = serde_json::json!([
        { "module_id": "sidebar", "version": "1.0.0",
          "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }],
          "requires": [] },
        { "module_id": "combat", "version": "1.0.0",
          "provides": [], "requires": ["shadowcat.surface:sidebar"] }
    ]);

    // A non-GM cannot read or write.
    pl.put(&format!("/api/worlds/{world_id}/contracts"))
        .json(&valid)
        .await
        .assert_status(StatusCode::FORBIDDEN);
    pl.get(&format!("/api/worlds/{world_id}/contracts"))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // The GM sets a valid set and reads it back.
    gm.put(&format!("/api/worlds/{world_id}/contracts"))
        .json(&valid)
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let got: serde_json::Value = gm
        .get(&format!("/api/worlds/{world_id}/contracts"))
        .await
        .json();
    assert_eq!(got[0]["provides"][0]["contract"], "shadowcat.surface:sidebar");

    // Dangling requires (no provider) is rejected.
    let dangling = serde_json::json!([
        { "module_id": "combat", "version": "1.0.0", "provides": [],
          "requires": ["shadowcat.surface:nonexistent"] }
    ]);
    gm.put(&format!("/api/worlds/{world_id}/contracts"))
        .json(&dangling)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Two singleton providers of the same contract is rejected.
    let dup_singleton = serde_json::json!([
        { "module_id": "a", "version": "1.0.0",
          "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }], "requires": [] },
        { "module_id": "b", "version": "1.0.0",
          "provides": [{ "contract": "shadowcat.surface:sidebar", "cardinality": "singleton" }], "requires": [] }
    ]);
    gm.put(&format!("/api/worlds/{world_id}/contracts"))
        .json(&dup_singleton)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // A malformed contract string is rejected.
    let malformed = serde_json::json!([
        { "module_id": "a", "version": "1.0.0",
          "provides": [{ "contract": "no-colon", "cardinality": "multi" }], "requires": [] }
    ]);
    gm.put(&format!("/api/worlds/{world_id}/contracts"))
        .json(&malformed)
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test contract_declarations_gm_crud_and_validation`
Expected: FAIL — route 404 / handlers not found.

- [ ] **Step 3: Implement validation + handlers**

Add to `src/server/src/http/routes.rs` (add `ContractDeclaration`, `Cardinality`
to the `crate::data::document` import at the top):

```rust
/// Upper bound on the number of contract declarations stored per world. Parsed
/// on every write and broadcast in `Welcome`; far above any realistic module set.
const MAX_CONTRACT_DECLARATIONS: usize = 256;

/// Structural validation of a contract id: `<namespace>:<name>`, both non-empty.
fn validate_contract_token(token: &str) -> Result<(), AppError> {
    match token.split_once(':') {
        Some((ns, name)) if !ns.is_empty() && !name.is_empty() => Ok(()),
        _ => Err(AppError::Unprocessable(format!(
            "malformed contract '{token}' (expected <namespace>:<name>)"
        ))),
    }
}

/// Validate a world's contract declaration set: bounded count, well-formed
/// non-empty fields, no duplicate `singleton` provider, and every `requires`
/// satisfied by some `provides` in the set. Fail-closed — the server is the
/// consistency authority.
fn validate_contract_declarations(decls: &[ContractDeclaration]) -> Result<(), AppError> {
    use std::collections::{HashMap, HashSet};
    if decls.len() > MAX_CONTRACT_DECLARATIONS {
        return Err(AppError::Unprocessable(format!(
            "too many declarations (max {MAX_CONTRACT_DECLARATIONS})"
        )));
    }
    let mut provided: HashSet<&str> = HashSet::new();
    let mut singleton_count: HashMap<&str, usize> = HashMap::new();
    for d in decls {
        if d.module_id.is_empty() || d.version.is_empty() {
            return Err(AppError::Unprocessable(
                "declaration module_id and version must be non-empty".into(),
            ));
        }
        for p in &d.provides {
            validate_contract_token(&p.contract)?;
            provided.insert(p.contract.as_str());
            if p.cardinality == Cardinality::Singleton {
                let n = singleton_count.entry(p.contract.as_str()).or_insert(0);
                *n += 1;
                if *n > 1 {
                    return Err(AppError::Unprocessable(format!(
                        "contract '{}' is singleton but provided more than once",
                        p.contract
                    )));
                }
            }
        }
    }
    for d in decls {
        for req in &d.requires {
            validate_contract_token(req)?;
            if !provided.contains(req.as_str()) {
                return Err(AppError::Unprocessable(format!(
                    "required contract '{req}' has no provider in the declared set"
                )));
            }
        }
    }
    Ok(())
}

/// A world's UI contract declarations. GM/admin only.
pub async fn get_world_contract_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
) -> Result<Json<Vec<ContractDeclaration>>, AppError> {
    require_gm(&state, &user, world).await?;
    Ok(Json(state.repo.world_contract_declarations(world).await?))
}

/// Replace a world's UI contract declarations. GM/admin only; validated.
pub async fn set_world_contract_declarations(
    user: AuthUser,
    State(state): State<AppState>,
    Path(world): Path<Uuid>,
    Json(decls): Json<Vec<ContractDeclaration>>,
) -> Result<StatusCode, AppError> {
    require_gm(&state, &user, world).await?;
    validate_contract_declarations(&decls)?;
    state
        .repo
        .set_world_contract_declarations(world, &decls)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Register the route**

In `src/server/src/http/mod.rs`, after the `capability-requirements` route:

```rust
        .route(
            "/api/worlds/{id}/contracts",
            get(routes::get_world_contract_declarations)
                .put(routes::set_world_contract_declarations),
        )
```

- [ ] **Step 5: Run it to verify it passes**

Run: `cargo test contract_declarations_gm_crud_and_validation`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/http/routes.rs src/server/src/http/mod.rs
git commit -m "feat(server): GM-only GET/PUT /api/worlds/{id}/contracts + validation"
```

---

### Task 3: `Welcome` broadcasts contract declarations

**Files:**
- Modify: `src/server/src/ws/protocol.rs` (add the field to `ServerMsg::Welcome`;
  update the two test constructions ~line 269 + the unit test)
- Modify: `src/server/src/ws/conn.rs` (load + populate in the egress task, ~line
  315-336)
- Generated: `src/types/generated/ServerMsg.ts` (via `cargo test --lib`)

**Interfaces:**
- Consumes: `Repository::world_contract_declarations` (Task 1).
- Produces: `ServerMsg::Welcome` gains
  `contract_declarations: Vec<crate::data::document::ContractDeclaration>`.

- [ ] **Step 1: Update the failing protocol test**

In `src/server/src/ws/protocol.rs`, edit `welcome_carries_caps_role_and_requirements`
to construct and assert the new field:

```rust
    #[test]
    fn welcome_carries_caps_role_and_requirements() {
        use crate::data::document::{CapabilityGrants, WorldRole};
        let w = ServerMsg::Welcome {
            world: Uuid::from_u128(1),
            current_seq: 0,
            server_time: 0,
            world_default_grants: CapabilityGrants::default(),
            actor_role: WorldRole::Player,
            capability_requirements: Vec::new(),
            contract_declarations: Vec::new(),
        };
        let json = serde_json::to_value(&w).unwrap();
        assert_eq!(json["type"], "welcome");
        assert_eq!(json["actor_role"], "player");
        assert!(json.get("world_default_grants").is_some());
        assert!(json.get("capability_requirements").is_some());
        assert!(json.get("contract_declarations").is_some());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test welcome_carries_caps_role_and_requirements`
Expected: FAIL — missing field `contract_declarations` in the `Welcome` literal
(compile error).

- [ ] **Step 3: Add the field to the protocol**

In `src/server/src/ws/protocol.rs`, add to the `Welcome` variant (after
`capability_requirements`, ~line 97):

```rust
        capability_requirements: Vec<crate::data::document::CapabilityRequirement>,
        /// The world's UI contract declarations, so the client can validate its
        /// loaded module set against the world's declared topology.
        contract_declarations: Vec<crate::data::document::ContractDeclaration>,
```

Find the OTHER `ServerMsg::Welcome { ... }` construction in this file's tests
(~line 269) and add `contract_declarations: Vec::new(),` to it.

- [ ] **Step 4: Populate it in the egress task**

In `src/server/src/ws/conn.rs`, after the `world_reqs` block (~line 324), load
the declarations (mirroring the warn-on-error pattern):

```rust
    let world_contracts = match repo.world_contract_declarations(world_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "contract declarations unreadable; sending empty");
            Vec::new()
        }
    };
```

Add the field to the `ServerMsg::Welcome { ... }` construction (~line 335):

```rust
            capability_requirements: world_reqs,
            contract_declarations: world_contracts,
```

- [ ] **Step 5: Run it to verify it passes + regenerate bindings**

Run: `cargo test welcome_carries_caps_role_and_requirements`
Then: `cargo test --lib` (regenerates `ServerMsg.ts` with the new field)
Expected: PASS; `src/types/generated/ServerMsg.ts` includes `contract_declarations`.

- [ ] **Step 6: Commit**

```bash
git add src/server/src/ws/protocol.rs src/server/src/ws/conn.rs \
        src/types/generated/ServerMsg.ts
git commit -m "feat(server): Welcome broadcasts world contract declarations"
```

---

### Task 4: Full-suite green + lint

**Files:** none (verification only)

- [ ] **Step 1: Run the whole server suite**

Run: `cargo test -p shadowcat`
Expected: PASS — all existing tests plus the three new ones green (lib + bin +
integration; the existing `ws_convergence`/`ws_live_search` handshakes still pass
with the added `Welcome` field).

- [ ] **Step 2: Confirm generated types are committed and in sync**

Run: `git status --porcelain src/types/generated/`
Expected: empty (the four `.ts` were committed in Tasks 1/3).

- [ ] **Step 3: Lint**

Run: `cargo clippy -p shadowcat --all-targets -- -D warnings`
Expected: no warnings.

---

## Self-Review

**Spec coverage (spec §3, §4):**
- Shared `ContractDeclaration` schema (ts-rs) → Task 1. ✓
- Settings-keyed storage, no migration (§4.1) → Task 1. ✓
- GM-only `GET/PUT /api/worlds/{id}/contracts` (§4.2) → Task 2. ✓
- Validation: count bound, dangling-requires, duplicate-singleton,
  malformed-contract, non-empty fields (§4.3) → Task 2 (`validate_contract_declarations`). ✓
- `Welcome` broadcast + warn-on-read-failure (§4.4) → Task 3. ✓
- Module-free invariant (§4.5) — declarations are strings only; no component or
  module-code handling added. ✓

**Placeholder scan:** No TBD/TODO; every code/test block is complete and runnable. ✓

**Type consistency:** `ContractDeclaration`/`ContractProvide`/`Cardinality`,
`world_contract_declarations`/`set_world_contract_declarations`,
`validate_contract_declarations`, the handler names, and the `Welcome` field
`contract_declarations` are used identically across all tasks. The
`Cardinality::Singleton` serde value `"singleton"` matches the Task-2 test JSON.
✓

## Out of scope (M7b-2 / M7b-3)

The client `ContributionRegistry`, manifest `provides`/`requires`, generalized
resolution, `Welcome`-topology reconciliation, and the Svelte `<Surface>` adapter
are later sub-milestones. M7b-1 ships only the server-side schema, storage,
endpoints, validation, and broadcast.
