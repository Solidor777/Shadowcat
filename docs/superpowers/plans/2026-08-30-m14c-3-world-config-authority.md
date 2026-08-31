# M14c-3 World-Config Authority Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: This plan is executed on a Fable-class session via
> `mainline-plan-execution` (user-directed). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move creation and definition of every world-config singleton from the GM's client to the
server: `create_world` + world-join seed all ten config singletons, engine defaults are defined in
Rust, `system-defaults` is server-written from the installed system package's manifest, and
reset-to-default becomes a clear (leaf-remove) intent.

**Architecture:** A new server-authored `WriteOrigin::ConfigSeed` commits seed ops built by one
shared ops-builder (`data::world_seed`), invoked from the world-create HTTP route
(via `apply_intent`), the WS world-join path (via `Room::publish`), and the enable-modules route
(system-defaults refresh). `WorldSettingsEngine` becomes an optional overlay sharing the
`SystemDefaultsEngine` member shapes; client seed paths are deleted and the client constants
become asserted mirrors.

**Tech Stack:** Rust (axum/sqlx/serde/ts-rs), Svelte 5 runes, TypeScript, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-08-30-m14c-3-world-config-authority-design.md`

## Model/Effort directives

Fable session; user directed mainline development. Plan written mainline; execution via
`mainline-plan-execution` in this session. No sdd-* dispatch for implementation. Reviews use
`shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`
(effort: high) per project CLAUDE.md.

## Buddy-check directives

- Task 1 (ConfigSeed origin + system-defaults ingress gate) is permission/authority-critical:
  buddy-check its diff (two blind reviewers + debate) before proceeding to Task 4.
- Final whole-branch review: the standard two-reviewer pair (spec-reviewer + code-reviewer),
  per the established M14c pattern.

## Global Constraints

- Never fork a decision across two paths: seed content and "what is missing" are computed in ONE
  ops-builder; callers differ only in commit transport (`apply_intent` vs `Room::publish`).
- No data migrations: no new migration files; `0001_init.sql` untouched (no schema change needed).
- No lint suppressions, no file-size allowlist entries; Rust test bodies in sibling files
  (`pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no ephemeral references (`pnpm lint:comments` gates).
- ts-rs: edit Rust, regenerate (`cargo test` exports), never hand-edit `src/types/generated`.
- `dist/` must exist before any cargo build (already built in this worktree).
- All doc-coverage gates are errors: every new pub item gets a real doc comment with `# Examples`
  where the crate requires it (`-D missing-docs`, `clippy::missing_docs_in_private_items`).
- Server crate is rustfmt-clean; run `cargo fmt` before each commit.

---

### Task 1: `WriteOrigin::ConfigSeed` + `system-defaults` ingress gate

**Files:**
- Modify: `src/server/src/data/command.rs` (WriteOrigin enum + is_server_authored)
- Modify: `src/server/src/data/sqlite.rs` (`apply_intent` Create/Update/Delete arms)
- Test: `src/server/src/data/sqlite/tests/commands_and_intents.rs`

**Interfaces:**
- Produces: `WriteOrigin::ConfigSeed` (server-authored; the only origin allowed to author
  `system-defaults` docs). Later tasks commit seed ops under it.

- [ ] **Step 1: Write failing tests** in `commands_and_intents.rs`: rewrite
  `system_defaults_is_a_singleton_and_gm_write_only` into three tests:
  `system_defaults_client_writes_are_rejected` (GM ctx, `WriteOrigin::Client`: Create rejected;
  after a `ConfigSeed` create, Update and Delete under `Client` rejected with
  `DataError::Forbidden`), `system_defaults_config_seed_writes_apply` (Create then Update under
  `ConfigSeed` succeed; singleton gate still rejects a second Create), and
  `config_seed_skips_capability_gates_but_not_occ` (mirror the existing `CombatTransition`
  test shape for the same properties).
- [ ] **Step 2: Run** `cargo test system_defaults` in `src/server` — expect FAIL
  (no `ConfigSeed` variant).
- [ ] **Step 3: Implement.** Add the variant:

```rust
/// Server-authored world-config seed/refresh write (world creation, world
/// join, enabled-modules change): per-op capability gates are skipped;
/// scope, size, engine, containment, singleton, schema and OCC checks all
/// run; never derivable from a wire frame. The ONLY origin permitted to
/// Create, Update or Delete a `system-defaults` doc.
ConfigSeed,
```

  Include it in `is_server_authored`. In `apply_intent`: in the `Create` arm reject
  `doc.doc_type == SYSTEM_DEFAULTS_DOC_TYPE && origin != WriteOrigin::ConfigSeed` with
  `DataError::Forbidden`; in the `Update` and `Delete` arms reject against the STORED doc_type
  (same authoritative-stored-type shape as the `MESSAGE_DOC_TYPE` guard beside it). Every
  per-op capability-gate site that currently branches on the server-authored origins follows
  `is_server_authored()` — verify no site matches variants individually (grep
  `CombatTransition` in `sqlite.rs`; any match-list found there gains the new variant only if
  it is NOT already `is_server_authored()`-driven).
- [ ] **Step 4: Run** `cargo test` (full crate) — expect PASS; `cargo fmt && cargo clippy`.
- [ ] **Step 5: Commit** `feat(data): WriteOrigin::ConfigSeed; system-defaults is server-authored`
- [ ] **Step 6: Buddy-check this diff** (two blind reviewers per the buddy-checking skill;
  reviewers get the pre-generated diff — they have no Bash). Fold fixes in before Task 4.

### Task 2: Engine seed bodies in Rust

**Files:**
- Modify: `src/server/src/data/engine/registries.rs`, `scene.rs`, `combat.rs` (seed constructors)
- Test: `src/server/src/data/engine/tests.rs`

**Interfaces:**
- Produces: `FactionRegistryEngine::seed()`, `ConditionRegistryEngine::seed()`,
  `ChannelRegistryEngine::seed()`, `LightGradationEngine::seed()`, `VisionModesEngine::seed()`
  (associated fns returning the default world content), plus existing `Default` impls for
  `ChatSettingsEngine`/`DiceSettingsEngine`/`ResourceRegistryEngine`/`SystemDefaultsEngine`
  (all-empty bodies — verify each derives/implements `Default`; add a derive where missing).

- [ ] **Step 1: Write failing tests** in `engine/tests.rs`: one test per `seed()` fn asserting
  the exact content (transcribed from the client constants, which these become the definition
  of): factions `friendly/neutral/hostile` with colors `#3fb950`/`#9e9e9e`/`#f85149` and
  matching stances; the nine conditions `dead 💀, unconscious 😵, prone 🛌, stunned 💫,
  poisoned 🤢, blinded 🙈, invisible 👻, hasted ⚡, slowed 🐌`; channels
  `general → Channel { name: "General" }`; gradation bands `bright 0.67 / dim 0.34 / dark 0.0`;
  vision modes `normal` (floor dim, range 0, no hint) and `darkvision` (floor dark, range 12,
  hint desaturate) — field names per the existing structs. Each seed body must pass
  `validate_engine` for its doc_type.
- [ ] **Step 2: Run** `cargo test engine` — expect FAIL.
- [ ] **Step 3: Implement** the `seed()` constructors (BTreeMap literals), e.g.:

```rust
impl FactionRegistryEngine {
    /// Default three-faction world seed (the engine definition; the client
    /// constants mirror this).
    pub fn seed() -> Self {
        let mut factions = BTreeMap::new();
        factions.insert("friendly".into(), Faction { name: "Friendly".into(), color: "#3fb950".into(), stance: FactionStance::Friendly });
        factions.insert("neutral".into(), Faction { name: "Neutral".into(), color: "#9e9e9e".into(), stance: FactionStance::Neutral });
        factions.insert("hostile".into(), Faction { name: "Hostile".into(), color: "#f85149".into(), stance: FactionStance::Hostile });
        Self { factions }
    }
}
```

  (Adjust field/variant spellings to the real structs.)
- [ ] **Step 4: Run** `cargo test engine` — PASS; `cargo fmt && cargo clippy`.
- [ ] **Step 5: Commit** `feat(engine): world-config seed bodies defined in Rust`

### Task 3: Manifest `systemDefaults` + system-provider detection + D6 guard

**Files:**
- Modify: `src/server/src/modules.rs` (mirror field, validation, provider detection)
- Modify: `src/server/src/http/module_routes.rs` (`set_world_enabled_modules` D6 check)
- Modify: `src/client/core/src/manifest.ts` (Zod `ManifestSchema` gains optional `systemDefaults`)
- Test: `src/server/src/modules/tests.rs`, `src/server/src/http/module_routes/tests.rs` (or the
  existing `module_routes` test module), `src/client/core/src/manifest.test.ts`

**Interfaces:**
- Produces: `InstalledModule.system_defaults: Option<SystemDefaultsEngine>`;
  `InstalledModule.provides_system: bool` (true when the raw manifest's `provides` array
  contains `"shadowcat.system"`); `set_world_enabled_modules` rejects >1 system provider.

- [ ] **Step 1: Write failing server tests**: a manifest with a valid `systemDefaults` object
  yields `Some` validated engine; an invalid one (unknown field / bad type) yields `None` and
  the module still loads; `provides: ["shadowcat.system"]` sets `provides_system`; the enable
  route returns 422 when two enabled ids both provide the system contract.
- [ ] **Step 2: Run** `cargo test modules` — expect FAIL.
- [ ] **Step 3: Implement**: `ModuleManifestMirror` gains
  `#[serde(default)] system_defaults: Option<serde_json::Value>`; at scan, decode it via
  `serde_json::from_value::<SystemDefaultsEngine>` + its `validate()` — on failure
  `tracing::warn!` and store `None` (fail-open discovery; the module itself still loads).
  Provider detection reads `manifest_json["provides"]` as an array of strings. In
  `set_world_enabled_modules`, after the engine-compat loop:

```rust
let systems: Vec<&str> = ids.iter()
    .filter(|id| installed.iter().any(|m| &m.id == *id && m.provides_system))
    .map(String::as_str).collect();
if systems.len() > 1 {
    return Err(AppError::Unprocessable(format!(
        "at most one enabled module may provide shadowcat.system (got: {})",
        systems.join(", ")
    )));
}
```

- [ ] **Step 4: Client Zod**: add `systemDefaults` to `ManifestSchema` as a passthrough-validated
  optional object (shape-check only — the server is the authority); Vitest case in
  `manifest.test.ts` that a manifest carrying it parses and retains the field.
- [ ] **Step 5: Run** `cargo test` + `pnpm --filter @shadowcat/core test` — PASS. `cargo fmt`.
- [ ] **Step 6: Commit** `feat(modules): manifest systemDefaults + single-system enable guard`

### Task 4: `data::world_seed` ops-builder + create-world seeding

**Files:**
- Create: `src/server/src/data/world_seed.rs` (+ `pub mod world_seed;` in `data/mod.rs`)
- Create: `src/server/src/data/world_seed/tests.rs` (sibling test file)
- Modify: `src/server/src/http/routes.rs` (`create_world` handler)
- Test: `src/server/src/http/tests.rs` (route-level)

**Interfaces:**
- Consumes: Task 2 seed bodies; Task 3 `InstalledModule.system_defaults`/`provides_system`.
- Produces:
  `pub fn missing_config_ops(existing: &[Document], world_id: Uuid, system_defaults: Option<&SystemDefaultsEngine>, now: i64) -> Vec<Operation>`
  — for each of the ten config doc types (`world-settings`, `vision-modes`, `light-gradation`,
  `chat-settings`, `dice-settings`, `channel-registry`, `faction-registry`,
  `condition-registry`, `resource-registry`, `system-defaults`), a Create when absent from
  `existing`; plus, when a `system-defaults` doc exists but its engine body differs from the
  declared `system_defaults` (or from the empty body when `None`), an Update on `/engine` with
  OCC pre-image. Also
  `pub async fn enabled_system_defaults(repo: &dyn Repository, world_id: Uuid, modules_dir: &Path) -> Option<SystemDefaultsEngine>`
  (reads the enabled set, scans installed modules, returns the single system provider's
  declared defaults — `None` when no system enabled or none declared) and
  `pub async fn seed_author(repo: &dyn Repository, world_id: Uuid) -> Option<PermissionContext>`
  (first GM member by sorted user id; `None` when the world has no GM).

- [ ] **Step 1: Write failing unit tests** (`world_seed/tests.rs`): empty `existing` yields ten
  Creates whose engine bodies equal the Task 2 seeds (world-settings/system-defaults empty
  overlay; resource-registry empty; dice/chat defaults); a full set yields zero ops; a set
  missing only `faction-registry` yields exactly that one Create; a stored `system-defaults`
  differing from the declared overlay yields one Update with the stored body as OCC pre-image;
  every Create's doc passes `validate_engine_tree`/engine validation and has
  `scope: Scope::World`, `owner: None`, default `PermissionSet` (mirror the envelope shape the
  client's `singleton_test_doc`/`world_scoped_doc` fixtures use — parentless, no name).
- [ ] **Step 2: Run** `cargo test world_seed` — FAIL.
- [ ] **Step 3: Implement** the builder (fresh `Uuid::new_v4()` ids; body via
  `serde_json::to_value(seed_body)`), `enabled_system_defaults`, `seed_author`.
- [ ] **Step 4: Wire `create_world`**: after `create_world_owned`, build ops against an empty
  `existing` slice with `enabled_system_defaults` (a brand-new world has no enabled modules —
  pass `None` without the scan) and commit via
  `repo.apply_intent(&ctx_of_creator, world.id, ops, now, WriteOrigin::ConfigSeed)`. A seed
  failure fails the request (world creation is atomic-in-effect: assert in a route test that a
  created world lists all ten config docs).
- [ ] **Step 5: Run** `cargo test` — PASS; `cargo fmt && cargo clippy`.
- [ ] **Step 6: Commit** `feat(data): world_seed ops-builder; create_world seeds config singletons`

### Task 5: Join-time reseed + enable-route system-defaults refresh

**Files:**
- Modify: `src/server/src/ws/conn.rs` (`handle_socket` after room obtained)
- Modify: `src/server/src/http/module_routes.rs` (`set_world_enabled_modules` refresh)
- Test: `src/server/src/ws/conn/tests/mod.rs` (join reseed), `module_routes` tests (refresh)

**Interfaces:**
- Consumes: Task 4 builder trio; `Room::publish`; `state.config.modules_path()`.

- [ ] **Step 1: Write failing tests**: (a) join-reseed — create a world through the repo directly
  (no singletons), run the join path (or a extracted `reseed_world_config(room, repo, modules_dir)`
  helper called by `handle_socket` — extract it so it is testable without a socket), assert all
  ten singletons exist after; running it twice adds nothing and returns Ok; a world with no GM
  member is a no-op Ok. (b) enable-route refresh — enable a fixture system module whose manifest
  declares `systemDefaults`; assert the stored `system-defaults` doc's engine now equals the
  declaration (Update with OCC), and disabling it refreshes back to the empty body.
- [ ] **Step 2: Run — FAIL.**
- [ ] **Step 3: Implement** `reseed_world_config`: `seed_author` → `enabled_system_defaults` →
  query the world's existing config docs (`repo` query by doc types) → `missing_config_ops` →
  if non-empty, `room.publish(repo, &author_ctx, ops, now, WriteOrigin::ConfigSeed)`, mapping a
  `DataError::Conflict` (lost seed race) to a logged no-op — a join must never fail on a lost
  race. Call it from `handle_socket` immediately after `get_or_create` succeeds, before the
  Welcome. In `set_world_enabled_modules`, after `set_world_enabled_modules` persists: room =
  `state.ws.rooms.get_or_create(...)`; run the same `reseed_world_config` (it covers the
  refresh case by construction — one decision path).
- [ ] **Step 4: Run** `cargo test` — PASS; `cargo fmt && cargo clippy`.
- [ ] **Step 5: Commit** `feat(server): join-time config reseed + enabled-system defaults refresh`

### Task 6: `world-settings` overlay conversion (server)

**Files:**
- Modify: `src/server/src/data/engine/scene.rs` (`WorldSettingsEngine` shape + Default),
  `src/server/src/data/engine/system_defaults.rs` (overlay types' doc comments now serve two
  owners), `src/server/src/data/engine/mod.rs` (normalization arm unchanged in name)
- Modify: `src/server/src/scene/mod.rs` (`validated_world_settings_engine` structural guard
  removal, `resolve_scene` world-layer folding), `src/server/src/combat/*` if
  `resolve_combat_rules` reads `WorldSettingsEngine.combat` (unchanged shape — verify only)
- Modify: `src/server/src/data/world_seed.rs` (world-settings seed body stays `{}` — now typed)
- Test: `src/server/src/scene/tests/resolution_and_lighting.rs`,
  `src/server/src/data/engine/tests.rs`

**Interfaces:**
- Produces: `WorldSettingsEngine { scene: Option<SceneDefaultsOverlay>, pathfinding:
  Option<PathfindingOverlay>, animation: Option<AnimationOverlay>, active_scene, combat }`
  (all `#[serde(default)]`, `Default` derived = empty overlay). Engine literals live on
  `impl Default for WorldSceneDefaults/Pathfinding/AnimationSettings` (moved from the old
  `WorldSettingsEngine::default` body) — the ONE innermost-fallback source (D8).

- [ ] **Step 1: Write failing tests**: engine tests — a partial `world-settings` body
  (`{"scene": {"fog": false}}`) validates; the old full-required body still validates (all
  fields optional ⊃ full); resolution tests — world overlay leaf beats system overlay leaf
  beats `WorldSceneDefaults::default()` literal, per representative leaf (fog,
  movement_model, diagonal_rule, speed_cells_per_sec); absent world-settings doc resolves
  identically to an empty one.
- [ ] **Step 2: Run — FAIL.**
- [ ] **Step 3: Implement**: restructure `WorldSettingsEngine`; move the literal bodies into
  `impl Default for WorldSceneDefaults` / `Pathfinding` / `AnimationSettings`; delete the
  structural triple-presence guard in `validated_world_settings_engine` (a partial doc decodes
  like system-defaults does); rewrite `resolve_scene`'s world layer to
  `ws_scene.and_then(|s| s.fog)` shape with `WorldSceneDefaults::default()` values as the
  innermost `unwrap_or` (bind `let d = WorldSceneDefaults::default();` once — no per-field
  literals). Update `WorldSettingsEngine::validate` (combat unchanged; animation/environment
  range checks now live on the shared overlay validation — reuse
  `SystemDefaultsEngine::validate`'s checks by extracting them to overlay-level `validate`
  fns both engines call). Update every server test constructing a full world-settings body
  only where it fails to compile.
- [ ] **Step 4: Run** `cargo test` — PASS (ts-rs exports regenerate under this run; leave
  `src/types/generated` changes uncommitted for Task 7); `cargo fmt && cargo clippy`.
- [ ] **Step 5: Commit** (server files only)
  `feat(engine,scene): world-settings is an optional overlay; engine literals single-sourced`

### Task 7: Client adaptation to the overlay shapes + reset-as-clear

**Files:**
- Modify: `src/types/generated/**` (commit the regenerated output)
- Modify: `src/client/core/src/scene-docs.ts` (`DEFAULT_WORLD_SETTINGS` retype + polarity flip,
  `buildWorldSettingsDoc`, `resolveSceneSettings`, `resolveSettingProvenance`/`resolvePick`)
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (reset + display reads)
- Test: `src/client/core/src/scene-docs.test.ts`,
  `src/modules/game-settings/src/GameSettingsPanel.test.ts` (or sibling tests), full
  `pnpm -r typecheck`

**Interfaces:**
- Consumes: regenerated `WorldSettingsEngine` (overlay), `WorldSceneDefaults`, `Pathfinding`,
  `AnimationSettings` types.
- Produces: `DEFAULT_WORLD_SETTINGS: { scene: WorldSceneDefaults; pathfinding: Pathfinding;
  animation: AnimationSettings }` (resolved-defaults shape, mirror of the server `Default`
  impls); `resolveSettingProvenance` reports `"world"` iff the leaf is PRESENT in the world
  doc (equality-collapse deleted); reset writes `null` to the leaf pointer.

- [ ] **Step 1: Regenerate + failing tests.** Run `cargo test` in `src/server` (exports ts-rs),
  `git status src/types/generated` to confirm the overlay shapes landed. Write/adjust Vitest
  cases: `resolveSceneSettings` folds engine < system < world per leaf with world leaves
  optional; a world doc `{scene:{fog:false}}` overrides; an empty world doc falls through to
  system then `DEFAULT_WORLD_SETTINGS`; provenance reports `world` only on presence (a world
  value equal to the system value but PRESENT is still `world` — assert the collapse is gone);
  `buildWorldSettingsDoc()` defaults to `{}` engine.
- [ ] **Step 2: Run** `pnpm --filter @shadowcat/core test` — FAIL.
- [ ] **Step 3: Implement** the client-core changes; `resolveSceneSettings` world reads become
  `world?.scene?.fog` etc.; delete `resolvePick`'s deep-equality collapse and the
  `SystemOrEngine` equality baseline note; keep `SystemOrEngine.value` as the reset TARGET
  only for display (the write is now a clear).
- [ ] **Step 4: Panel.** `resetToSystem(path, old)` dispatches a set of `null` at the leaf
  pointer (`/engine/scene/fog` etc.) with the stored value as OCC pre-image (the established
  null-⇒-inherit wire convention); world-defaults inputs display the provenance-resolved
  effective value (they already render `provControl` per path); remove the
  "REQUIRED on the wire" comment block and `WorldDefaultsPath`'s reset-writes-literal rationale.
- [ ] **Step 5: Run** `pnpm -r test && pnpm -r typecheck` — PASS.
- [ ] **Step 6: Commit** `feat(client): overlay world-settings; reset clears the leaf`

### Task 8: Delete client seed paths + `Module.systemDefaults`

**Files:**
- Modify: `src/client/shell/src/lib/worldSession.svelte.ts` (drop the `#onWelcome`
  system-defaults upsert + import), `src/client/core/src/modules.ts` (drop
  `Module.systemDefaults`), `src/client/core/src/scene-docs.ts` (delete
  `systemDefaultsUpsertOps`, `seedResourceRegistryIfAbsent`; keep `buildSystemDefaultsDoc`
  only if a test still constructs fixtures with it — otherwise delete),
  `src/client/core/src/index.ts` (export list)
- Modify: `src/modules/game-settings/src/GameSettingsPanel.svelte` (delete the five-singleton
  seed effect), `src/modules/chat/src/ChatPanel.svelte` (delete the registry seed effect),
  `src/modules/factions/src/FactionsPanel.svelte` + `src/modules/factions/src/seed.ts` (delete
  file), `src/modules/conditions/src/ConditionsPanel.svelte` +
  `src/modules/conditions/src/seed.ts` (delete file)
- Test: the corresponding module/unit tests (`GameSettingsPanel`, `ChatPanel`, `FactionsPanel`,
  `ConditionsPanel`, `worldSession`, `scene-docs`) — seed assertions become
  no-seed assertions; panels get their registry docs from fixture stores.

**Interfaces:**
- Consumes: server-side seeding (Tasks 4-5) as the replacement.

- [ ] **Step 1: Delete** the seed paths and constants listed above. File deletions use
  `git restore`-safe flow per the deletion rule: `trash <path>` then `git add` the removal
  (never `git rm` as the sole step, never `rm`).
- [ ] **Step 2: Update tests**: replace "seeds once for GM when absent" cases with "never
  dispatches a create" cases; panel fixtures pre-populate their registry docs (use the
  builders/fixtures already in each test file). `worldSession.test.ts` drops the upsert cases.
- [ ] **Step 3: Run** `pnpm -r test && pnpm -r typecheck && pnpm lint` — PASS.
- [ ] **Step 4: Commit** `refactor(client): remove client-side config seeding`

### Task 9: Prose polarity + guides + ARCHITECTURE

**Files:**
- Modify: `src/client/core/src/scene-docs.ts` (`DEFAULT_WORLD_SETTINGS`/`DEFAULT_GRADATION`/
  `SEED_VISION_MODES` doc comments: mirror-of-server), `src/server/src/data/engine/scene.rs`
  (`Default` impl comments: definition, client mirrors), `src/server/src/data/engine/
  system_defaults.rs` (module + struct docs: server-written from the manifest),
  `src/server/src/scene/tests/resolution_and_lighting.rs` ("mirrors client default" wording)
- Modify: `docs/design/ARCHITECTURE.md` (the four-tier-chain sentence describing the client
  upsert; world-config authority statement), `docs/site/guides/creating-a-system.md`
  (declare defaults via manifest `systemDefaults`), `docs/site/guides/creating-a-module.md` +
  the wire-protocol/module portal pages ONLY where they name the manifest shape (grep
  `systemDefaults` under `docs/site/`)

- [ ] **Step 1: Flip** every authority comment found by
  `grep -rn "authoritative source\|MUST equal the client\|mirrors client default" src/` —
  server is the definition; client constants are asserted mirrors. Keep the parity tests,
  reword their assertion messages off the old polarity.
- [ ] **Step 2: Rewrite** the ARCHITECTURE sentence(s) (grep `system-defaults` and
  `four-tier` / `upsert` in `docs/design/ARCHITECTURE.md`) and the creating-a-system guide's
  defaults section to the manifest field.
- [ ] **Step 3: Run** `pnpm lint:comments && pnpm lint:docs && pnpm lint:props` and
  `cargo test` — PASS.
- [ ] **Step 4: Commit** `docs: server defines world-config defaults; client mirrors`

### Task 10: Full gates + e2e

- [ ] **Step 1: Gate sweep** (paste-and-run, read each output): `src/server`: `cargo fmt --check
  && cargo clippy -- -D warnings && cargo test`; repo root: `pnpm -r test`, `pnpm -r
  typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`,
  `pnpm lint:file-size`, `pnpm lint:inline-tests`, `pnpm lint:allowances`,
  `pnpm docs:check-examples`, `pnpm run test:scripts`, `pnpm build`.
- [ ] **Step 2: E2E check** — the shell e2e reuses ANY server on fixed port 31999 and a parallel
  session (m15b) may be running one: coordinate/rerun alone. Add/extend a server-side
  integration test (`test_server` harness or `http/tests.rs`) proving: create world via HTTP →
  all ten config singletons queryable without any client dispatch; enable a system module
  fixture → `system-defaults` updates.
- [ ] **Step 3: Fix forward** anything red (systematic-debugging on any failure; no suppressions,
  no descopes). Commit fixes as their own logical units.

### Task 11: Docs sync, skills, memory, merge

- [ ] **Step 1: Tracking docs**: `docs/PLAN.md` (M14c-3 → done marker), `docs/HISTORY.md`
  (M14c-3 delivery entry), `docs/TODO.md` (nothing deferred, or entries for anything the user
  approves deferring), spec amendments if execution deviated.
- [ ] **Step 2: Skill updates** in `~/.claude/skills/shadowcat-codebase/skills/`:
  `documents-permissions` (ConfigSeed origin, system-defaults server-only),
  `client-shell` (Welcome no longer upserts; seeds gone), `module-toolchain` (manifest
  `systemDefaults`, single-system enable rule), `server-ops` only if config/CLI touched.
  Dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff; run
  `node scripts/check-skill-symbol-refs-cli.mjs` + `pnpm run test:scripts`; bump the plugin
  `version` in `.claude/.claude-plugin/plugin.json` (plugin checkout), commit + push the
  plugin repo.
- [ ] **Step 3: Final branch review**: two-reviewer pair (spec-reviewer + code-reviewer, high
  effort) over the whole branch diff vs main; fix findings; re-run Step 1 of Task 10 if code
  changed.
- [ ] **Step 4: Merge** `--no-ff` into main, run both suites on main, push, `gh run watch`.
  Update memory (`m14c-server-authority-campaign-state.md` → M14c-3 done, next M14c-4).
