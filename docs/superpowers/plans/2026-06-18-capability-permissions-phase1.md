# Capability Permissions — Phase 1 Implementation Plan

> Executed via `mainline-plan-execution` (Opus-class): inline TDD per task, a
> per-task enumerative spec-compliance check, and ONE dispatched final branch
> review. Branch: `capability-permissions-phase1` off `main`.
> Spec: `docs/superpowers/specs/2026-06-18-capability-permissions-design.md`.

**Goal:** Replace M5's binary `can_write`/`can_read` with a namespaced
capability model (`core:*`), enforced through `apply_intent` via a path →
capability map; grants widen a built-in `DocRole` floor additively, per-document
and per-world-default. No DB migration (additive `serde(default)` field).

**Approved decisions:** world-level `core:create` (GM always allowed); world
defaults keyed by world × optional `doc_type`; additive-only grants; `by_role` +
`by_user` grant maps; per-document grants + path map first, world defaults next.

## Global constraints

- Server-authoritative, structural validation only; GM omnipotent; one write
  path (`apply_intent`); ordered realtime; ts-rs bindings CI-enforced.
- Commit trailers on every commit:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Htozbntnxh8N3meNWAeoNp
  ```

---

### Task 1: Capability types + capability-aware resolver

**Files:** `src/server/src/data/document.rs` (CapabilityGrants on PermissionSet),
`src/server/src/data/permission.rs` (cap consts, floor table, `Access`,
`resolve_access`, `filter_properties`, `filter_command`).

- [ ] **Step 1 — types.** In `document.rs`, add the grants struct and field:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
  #[ts(export, export_to = "../../types/generated/")]
  pub struct CapabilityGrants {
      #[serde(default)] pub by_role: BTreeMap<DocRole, BTreeSet<String>>,
      #[serde(default)] pub by_user: BTreeMap<Uuid, BTreeSet<String>>,
  }
  ```
  Add `#[serde(default)] pub capabilities: CapabilityGrants,` to `PermissionSet`.
  (BTreeMap with DocRole key needs DocRole: Ord — derive `PartialOrd, Ord` on
  DocRole.)
- [ ] **Step 2 — capability constants.** In `permission.rs`:
  ```rust
  pub mod cap {
      pub const READ: &str = "core:read";
      pub const WRITE_FIELDS: &str = "core:write_fields";
      pub const MANAGE_EMBEDDED: &str = "core:manage_embedded";
      pub const DELETE: &str = "core:delete";
      pub const EDIT_PERMISSIONS: &str = "core:edit_permissions";
      pub const CREATE: &str = "core:create";
  }
  ```
- [ ] **Step 3 — `Access` + resolver.** Replace `Access` with:
  ```rust
  pub struct Access { pub caps: BTreeSet<String>, pub all: bool, pub see_gm_only: bool }
  impl Access { pub fn has(&self, c: &str) -> bool { self.all || self.caps.contains(c) } }
  ```
  `resolve_access(user, world_role, doc) -> Access`: GM ⇒ `{caps: empty, all: true,
  see_gm_only: true}`. Else compute the actor's `DocRole` (per-user else default),
  seed caps from the built-in floor (Owner→{READ,WRITE_FIELDS}, Observer→{READ},
  None→{}), then union the document's additive grants `by_role[role]` and
  `by_user[user]`. `see_gm_only = false`.
- [ ] **Step 4 — update consumers.** `filter_properties` uses `access.see_gm_only`
  (unchanged). `filter_command`: replace `access.can_read` with
  `access.has(cap::READ)`; replace the `see_gm_only` branch unchanged.
- [ ] **Step 5 — tests** (write first, watch fail): floor (Owner has
  WRITE_FIELDS, not MANAGE_EMBEDDED; Observer read-only; None none); GM `all`;
  additive `by_role` grant adds MANAGE_EMBEDDED to Owner; `by_user` grant; the
  existing GmOnly filter tests still pass with `has(READ)`.
- [ ] **Step 6 — build + fix call sites** (`apply_intent`, http routes still
  reference `can_read`/`can_write` — they are rewritten in Task 2/Task 4; to keep
  this commit compiling, do the minimal mechanical swap `can_read→has(cap::READ)`
  and `can_write→has(cap::WRITE_FIELDS)` at read sites now, full op-gating in T2).
- [ ] **Step 7 — commit.** `feat(cap): namespaced capability model + resolver`.

---

### Task 2: apply_intent path → capability gating

**Files:** `src/server/src/data/sqlite.rs` (`apply_intent`), `permission.rs`
(a `required_cap_for_path` helper).

- [ ] **Step 1 — path→cap helper** in `permission.rs`:
  ```rust
  /// The capability required to write `path`, or None if the envelope field is
  /// immutable via Update.
  pub fn required_cap_for_path(path: &str) -> Option<&'static str> {
      if path == "/system" || path.starts_with("/system/") { Some(cap::WRITE_FIELDS) }
      else if path == "/embedded" || path.starts_with("/embedded/") { Some(cap::MANAGE_EMBEDDED) }
      else if path == "/permissions" || path.starts_with("/permissions/") { Some(cap::EDIT_PERMISSIONS) }
      else { None }
  }
  ```
- [ ] **Step 2 — failing tests** in `sqlite.rs`: replace
  `apply_intent_rejects_envelope_patch` expectations — `/permissions/default`
  with `core:edit_permissions` granted now SUCCEEDS; without it → Forbidden.
  Owner with `core:manage_embedded` granted can add an `/embedded/...` entry;
  Owner without it → Forbidden. Owner can still write `/system`. `/id` etc still
  rejected.
- [ ] **Step 3 — implement.** In `apply_intent` Update authorize: for each
  change, `let need = required_cap_for_path(&ch.path).ok_or(Forbidden)?; if
  !access.has(need) { return Err(Forbidden); }` where `access = resolve_access(
  ctx.user_id, ctx.world_role, &cur)`. Create: require `access.has(WRITE_FIELDS)`
  on the new doc (creator authorship) — keep existing can_write semantics via
  WRITE_FIELDS. Delete: require `access.has(DELETE)`. Remove the hard-coded
  `/system`-only check (now subsumed by the path map). Keep `validate_field_path`,
  size cap, scope checks, id-immutability, pre-image OCC.
  > Note: `core:delete` is NOT in the built-in floor, so by default only GM
  > deletes — a behavior change from M5 (Owner could delete). Intended per spec;
  > grant `core:delete` to restore owner-delete where desired. Call this out in
  > the task compliance check.
- [ ] **Step 4 — apply phase** already merges `/system`; ensure `/embedded` and
  `/permissions` changes merge through the same `set_pointer` path (they do — the
  apply phase is path-agnostic; only authorize gated them).
- [ ] **Step 5 — run tests; commit.** `feat(cap): apply_intent path→capability gating + embedded/ACL ops`.

---

### Task 3: World-default capability grants

**Files:** `permission.rs` (resolution takes world defaults), `sqlite.rs`
(load/store world defaults), `data/mod.rs` if a new error is needed.

- [ ] **Step 1 — storage.** Store world defaults as a JSON settings record keyed
  `world_caps:<world_id>` (reuse the `settings` table via `get_setting`/
  `set_setting`): a `BTreeMap<Option<String>, CapabilityGrants>` (doc_type →
  grants; `None` = all types). Add `SqliteRepository::world_capability_defaults(
  world) -> Result<WorldCapDefaults, DataError>` and a setter.
- [ ] **Step 2 — thread into resolution.** Extend `PermissionContext` with the
  resolved world defaults (loaded in `permission_context`), or pass them to a new
  `resolve_access_with_defaults(ctx, defaults, doc)`. Prefer carrying a resolved
  `BTreeSet<String>` floor-addition for the actor's role+doc_type in the ctx to
  keep `resolve_access` cheap. Layer precedence per spec §4.
- [ ] **Step 3 — tests:** world default granting Owners `core:manage_embedded`
  applies to a doc with no per-doc grant; per-doc grant still layers; doc_type
  scoping respected.
- [ ] **Step 4 — commit.** `feat(cap): world-default capability grants`.

---

### Task 4: HTTP/WS grant management + create authorization

**Files:** `src/server/src/http/routes.rs`, `http/mod.rs`.

- [ ] **Step 1 — create authorization.** `create_document` / WS create: require
  `core:create` at world level (GM always allowed). Resolve via world defaults /
  a world grant; deny otherwise (403/Forbidden reject).
- [ ] **Step 2 — grant endpoints.** `PUT /api/documents/{id}/permissions`
  (set the document's `CapabilityGrants`, gated by `core:edit_permissions` or GM);
  `PUT /api/worlds/{id}/capability-defaults` (GM/admin). Both validate capability
  strings are well-formed (`<ns>:<verb>`), structural only.
- [ ] **Step 3 — tests** (axum-test): player-owner rolls (200) but cannot manage
  embedded (403) until granted; GM grants `core:manage_embedded` per-doc → owner
  succeeds; non-GM cannot edit grants; create gated by `core:create`.
- [ ] **Step 4 — commit.** `feat(cap): grant-management endpoints + create authz`.

---

### Task 5: Telemetry, lint, docs, bindings

- [ ] `tracing::debug!` on capability denials (reason = missing capability).
- [ ] `cargo fmt --all`; `cargo clippy --all-targets -- -D warnings`.
- [ ] `cargo test --all`; `git diff --exit-code src/types/generated` (PermissionSet,
  CapabilityGrants bindings regenerated).
- [ ] Mark the spec Phase 1 as implemented; note Phase 2 (M6) remains; update
  `POST_WORK_FINDINGS.md` if anything surfaces. Confirm M5 docs (`core:delete`
  default-GM behavior change) noted.
- [ ] Commit. Then final dispatched branch review → finishing-a-development-branch.

## Self-review

- Spec §3.1 path map → T2 `required_cap_for_path`. §4 grants → T1 (floor + doc) +
  T3 (world default). §7.1 create → T4. Backward-compat (serde default) → T1.
  Embedded operation gap (M5) closed → T2. ACL edits gated → T2/T4.
- **Behavior change to flag:** `core:delete` not in floor → owners can no longer
  delete by default (M5 allowed it). Intended; grantable. Surface in review.
- Phase 2 (module-declared caps, action hooks) intentionally excluded.
