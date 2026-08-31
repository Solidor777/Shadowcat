# M14c-3 — World-Config Authority

## 0. Problem

Every world-scoped config singleton is created by the GM's **client**, each from a different
place, and several defaults are documented as client-authoritative with the server required to
mirror them. That inverts the server-authoritative invariant (ARCHITECTURE §2) and is the
fork-a-decision class: the definition of "a world's initial configuration" is currently spread
across five client seed sites, two constant families (client TS + server Rust), and a stored doc
(`system-defaults`) whose content is declared in client module *code* the server cannot read.

Inventory of client authority to remove (audit groups 2+3 of the M14c umbrella,
`2026-08-30-m14c-1-server-formula-engine-design.md` Appendix A):

| Site | What it seeds/decides |
|---|---|
| `GameSettingsPanel` mount effect | creates `world-settings`, `light-gradation`, `vision-modes`, `dice-settings`, `chat-settings` |
| `ChatPanel` mount effect | creates `channel-registry` |
| `FactionsPanel` / `ConditionsPanel` mount effects | create `faction-registry` / `condition-registry` from module-local `SEED` constants |
| `WorldSession.#onWelcome` (GM) | upserts `system-defaults` from `Module.systemDefaults` — an in-code field on the client `Module` object, deliberately absent from the Zod manifest |
| `seedResourceRegistryIfAbsent` (`@shadowcat/core`) | exported seed helper for `resource-registry` (no production caller; tests only) |
| `DEFAULT_WORLD_SETTINGS` / `SEED_VISION_MODES` / `DEFAULT_GRADATION` | documented "the client stays the authoritative source"; the server's `WorldSettingsEngine::default` is documented "MUST equal the client's" |
| `GameSettingsPanel.resetToSystem` | reset writes a **client-resolved literal** of the layer beneath, because `WorldSettingsEngine` leaves are required-on-wire; provenance (`resolveSettingProvenance`) needs a deep-equality collapse to guess whether a world value is a real override |

## 1. Decisions

| # | Decision |
|---|---|
| D1 | **`create_world` seeds every config singleton server-side.** One repository helper (`seed_world_config`, name indicative) creates any absent config singleton for a world through the normal command pipeline (`apply_intent`), under a new server-authored `WriteOrigin::ConfigSeed`. Called from `create_world_owned` (author = the creator) and from `RoomRegistry::get_or_create` hydration (lazy reseed-if-absent; author = the world's first GM by deterministic order — see D7). One code path for both callers; idempotent (creates only what is absent). |
| D2 | **Engine defaults are defined in Rust.** The seed contents for `faction-registry`, `condition-registry`, `channel-registry`, `resource-registry`, `vision-modes`, `light-gradation`, `chat-settings`, `dice-settings` become server-crate constants/`Default` impls (transcribed from today's client SEED constants — behavior-identical content). Client constants that survive for preview/UI (`DEFAULT_WORLD_SETTINGS`, `DEFAULT_GRADATION`, `SEED_VISION_MODES`) flip documentation polarity to "mirror of the server, asserted by the existing parity tests"; client SEED constants with no remaining consumer are deleted with their seed helpers. |
| D3 | **`world-settings` becomes an optional overlay.** `WorldSceneDefaults`/`Pathfinding`/`AnimationSettings` leaves become `Option`-lifted, reusing the same overlay member shapes `SystemDefaultsEngine` already has (`SceneDefaultsOverlay`/`PathfindingOverlay`/`AnimationOverlay`) so the world and system layers share one shape instead of two parallel type families. `create_world` seeds an **empty** `world-settings` doc. The write-time structural-completeness guard is removed; resolution falls through world → system → engine per leaf on both sides. |
| D4 | **Reset-to-default is a clear intent.** The client resets a setting by removing the stored leaf (JSON-pointer remove / null per existing wire semantics), never by writing a resolved literal. Provenance becomes structural: a leaf present in the world doc IS a world override; `resolvePick`'s deep-equality collapse is deleted. |
| D5 | **`system-defaults` is server-written, from the manifest.** `module.json` gains an optional `systemDefaults` field, validated server-side against `SystemDefaultsEngine` at scan (invalid ⇒ warn + treat as absent, matching fail-open discovery). The server upserts the stored `system-defaults` singleton from the enabled system package's manifest at: world seed (D1), and `set_world_enabled_modules`. Client writes to `system-defaults` are blanket-rejected (`WriteOrigin::ConfigSeed` exempt); `Module.systemDefaults` (code-level), `systemDefaultsUpsertOps`, and the `#onWelcome` upsert are deleted. The client Zod `ManifestSchema` mirrors the new field for authoring-time validation. |
| D6 | **At most one enabled system per world.** `set_world_enabled_modules` rejects an enabled set in which more than one installed module's manifest `provides` contains `shadowcat.system` — otherwise the server's system-defaults pick and the client's `SYSTEM_CONTRACT` winner could diverge (a fork on "which system is active"). First-party in-code modules cannot be systems after D5 (none are today). |
| D7 | **Seed authorship.** `Command.author` stays a required `Uuid` (no wire change). `create_world_owned` attributes seeds to the creator. The hydration reseed attributes to the world's first GM member in deterministic (sorted-by-user-id) order; a world with no GM member skips the reseed (nothing to attribute; `create_world_owned` always seats one, so this arises only in legacy/test fixtures). |
| D8 | **Engine-literal fallbacks read one shared symbol.** `SceneEcs::resolve_scene`'s inline `unwrap_or(...)` literals are replaced by reads of `WorldSettingsEngine::default()`-derived values (one source), removing the existing intra-server fork between the `Default` impl and the per-field literals. Client `resolveSceneSettings` keeps reading `DEFAULT_WORLD_SETTINGS` (the asserted mirror). |

## 2. Server design

### 2.1 `WriteOrigin::ConfigSeed`

New variant beside `CombatTransition`: server-authored (`is_server_authored() == true`), skips
per-op capability gates, keeps structural/OCC/engine/singleton checks. Additionally it is the
ONLY origin allowed to Create or Update a `system-defaults` doc; `Client` writes targeting
`system-defaults` are rejected at ingress (same blanket-rule shape as the stored-`message`
Update guard, gated in `apply_intent`).

### 2.2 `seed_world_config`

For each config singleton doc type in one fixed list — `world-settings`, `vision-modes`,
`light-gradation`, `chat-settings`, `dice-settings`, `channel-registry`, `faction-registry`,
`condition-registry`, `resource-registry`, `system-defaults` — create it iff absent, with the
Rust default body (D2/D3; `world-settings` seeds empty-overlay, `system-defaults` seeds from the
enabled system's manifest `systemDefaults` if one is enabled, else empty). In the same pass, when a stored `system-defaults` body differs
from what the enabled system's manifest declares, it is content-refreshed (an Update with OCC
pre-image) — the stored copy is a server-owned mirror of the manifest and must not drift. Ops go
through `apply_intent` (or `Room::publish` when a live room must broadcast) under `ConfigSeed`.
Seeded docs use fresh UUIDs: absence/lookup is by `doc_type`, and the singleton ingress guard
backstops any server-internal race, so no client-mirroring deterministic-id derivation is
needed.

Call sites:
- `create_world_owned` (after seating the GM, same flow) — author = creator.
- `RoomRegistry::get_or_create` hydration — lazy reseed for pre-existing dev worlds and
  self-healing after a deletion (a deleted singleton resurrects empty on next room open).
- `set_world_enabled_modules` — after validating D6, refresh `system-defaults` content from the
  (possibly changed) system package: an Update with OCC pre-image when content differs, going
  through `RoomRegistry::get_or_create` + `Room::publish` so a live room broadcasts (one path,
  not an if-room-open fork).

A manifest edited on disk with no enable-set change is picked up at the next room hydration;
that staleness window is accepted and documented.

### 2.3 Manifest surface

`ModuleManifestMirror` gains `#[serde(default)] system_defaults: Option<serde_json::Value>`;
`InstalledModule` carries the validated `Option<SystemDefaultsEngine>` (validation via the
existing `SystemDefaultsEngine` deserialization; failure ⇒ warn + `None`). The server determines
"the system" of an enabled set by parsing each enabled module's raw manifest `provides` array
for `"shadowcat.system"` (the same contract id the client's `SYSTEM_CONTRACT` names).

### 2.4 Overlay `world-settings`

`WorldSettingsEngine` becomes `{ scene: SceneDefaultsOverlay, pathfinding: PathfindingOverlay,
animation: AnimationOverlay, active_scene, combat }` with every member `#[serde(default)]` —
the same overlay member types `SystemDefaultsEngine` uses (moved/shared, not duplicated).
`validated_world_settings_engine`'s structural triple-presence guard is deleted;
`resolve_scene` folds world/system per leaf with `WorldSettingsEngine`-default innermost
fallbacks via one shared source (D8). ts-rs regen propagates the shape to
`src/types/generated`; the client Zod mirror follows.

## 3. Client design

Deletions: `GameSettingsPanel` seed effect; `ChatPanel` registry seed; `FactionsPanel`/
`ConditionsPanel` seeds + module `SEED` constants (content now lives in Rust);
`seedResourceRegistryIfAbsent`; `systemDefaultsUpsertOps`; `Module.systemDefaults`; the
`#onWelcome` system-defaults upsert. The GM first-scene seed in `#onWelcome` **stays** (a scene
is not config; out of scope).

Changes: `buildWorldSettingsDoc` builds the empty overlay (no full-default seeding);
`resolveSceneSettings`/`resolveSettingProvenance` read optional world leaves (presence =
override; equality-collapse deleted); `GameSettingsPanel.resetToSystem` issues a leaf-remove
(clear) instead of writing a literal, and its per-leaf "REQUIRED on the wire" rationale comment
goes; `ManifestSchema` gains optional `systemDefaults`. Client builders retained where tests or
UI still construct docs (now matching the new shapes).

Prose polarity flip on `DEFAULT_WORLD_SETTINGS`/`DEFAULT_GRADATION`/`SEED_VISION_MODES` and the
server `Default` impls: the server is the definition; the client constant is the asserted mirror
(existing parity tests keep both honest; direction of authority in the comments flips).

## 4. Security / permissions

- `system-defaults`: was GM-write-only; becomes server-write-only (`ConfigSeed`).
- Other config singletons: seeding moves server-side; GM **editing** stays exactly as today
  (the settings UIs keep writing field updates under `Client` origin with capability gates).
- `set_world_enabled_modules` keeps its GM/admin gate and adds the D6 single-system check.
- No egress/visibility change to any config doc.

## 5. Testing

- Server: seed-on-create (all ten singletons exist with expected bodies); hydration reseed
  idempotence + only-absent creation + no-GM skip; `ConfigSeed` gate (client Create/Update of
  `system-defaults` rejected; `ConfigSeed` accepted); D6 rejection; enable-route refresh
  updates content with OCC; overlay `world-settings` resolution per leaf (world < system <
  engine) incl. absent-doc and partial-doc cases; manifest `systemDefaults` scan (valid /
  invalid-warn-absent); parity tests updated to assert client-mirrors-server.
- Client: seed effects removed (panels no longer dispatch creates); reset issues a remove and
  provenance reports structurally; Zod/ts-rs drift guard passes on the regenerated shapes.
- E2E (`test_server` harness where it fits): create world → connect → config singletons present
  without any client dispatch; enable a system module → `system-defaults` doc updates.

## 6. Documentation & follow-through

- ARCHITECTURE: the four-tier-chain sentence that still describes the client upsert is
  rewritten to the server-authored flow; invariant prose on world-config authority.
- `creating-a-system.md`: declare defaults via manifest `systemDefaults` (code-level field
  removed); `creating-a-module.md`/wire-protocol page if they name the manifest shape.
- PLAN/HISTORY/TODO sync; skill updates via the reviewed gate (`documents-permissions`,
  `client-shell`, `module-toolchain`, `server-ops` as touched); memory state update.

## 7. Out of scope

M14c-4/5/6 items; the GM first-scene seed; FTS/search; any data migration (no-migrations
directive — dev DBs predating this are covered by the lazy reseed, or deleted).
