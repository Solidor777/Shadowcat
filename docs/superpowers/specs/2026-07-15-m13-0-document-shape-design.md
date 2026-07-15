# M13-0 · Three-Band Document Shape — Design Spec

Status: **approved design, ready for implementation planning** (user-approved 2026-07-15).
Parent decision: Nightfox spec D15 (`2026-07-15-m13-nightfox-system-design.md`). This
checkpoint restructures the document body BEFORE any Nightfox code depends on it.

## 1. Goal & motivation

Split the document's opaque `system` body by **schema ownership**. Today engine-read fields
(token transform, scene grid/vision/lighting, wall flags, region geometry, actor
vision/conditions/…) squat the `system` root alongside future game-system data, and every
body shape is a hand-mirrored, comment-coupled type maintained in **three places** (server
pointer walks, `scene-docs.ts` interfaces, redundant render-view locals) with zero machine
drift-guard below the envelope. M13-0:

1. gives the engine's game data a typed, ts-rs-generated home (`engine`), ending the
   hand-mirror drift risk structurally;
2. hands `system` to the world's (singleton) game system outright — `system.stats` /
   `system.mechanics` (Nightfox D13/D14) never collide with engine fields;
3. moves the one universal identity field (`name`) into the envelope.

Hard cutover, **zero migration code** — pre-v1, no shipped worlds exist (M2
no-migrations-in-v1 stance holds).

## 2. Decisions

| # | Decision |
|---|---|
| S1 | **Composition rule** (user decision): a document is composed from **envelope + `engine`? + `system`**. `engine` exists **iff** the doc_type is engine-defined; community/system doc types have **no `engine` key at all** (not an empty one). A write carrying `engine` on a non-engine doc_type is rejected. |
| S2 | **`name` is an envelope field** (user decision): `name: Option<String>` on every document. Browsers/search/folders read one place; ts-rs drift-guards it. Per-type aliases stay engine-band (actor `displayName`, token `overrides.name`). Name privacy override pointer becomes `/name`. |
| S3 | **Strict ingress** (user decision): an `engine` write for an engine doc_type must deserialize into that type's Rust struct (`deny_unknown_fields`); type/range/unknown-field violations are rejected at the intent like envelope violations. Fail-closed read defaults remain as backstop. |
| S4 | **Reserved fourth band `modules`** (user decision): documents will carry a generic catch-all directory for module-specific persisted data that is neither engine nor game-system territory — `modules: Record<moduleId, unknown>`, engine-opaque, structurally validated only. **NOT implemented in M13-0**; the root key is reserved here so nothing else squats it, and the typed envelope (which rejects unknown root keys) enforces the reservation for free until it ships. |
| S5 | **Wire/Zod stance**: the client wire schema keeps `engine: z.unknown()`; the server's strict ingress is the runtime guarantee and the generated `*Engine` TS types are the compile-time contract. No per-doc-type client Zod for `engine`. (`system` tier-1 Zod remains the game system's job — Nightfox M13b.) |
| S6 | **Caps**: the existing 256 KiB serialized-size cap applies **per block** to `engine` and to `system` independently (region/drawing point arrays make `engine` size-unbounded without it). Same constant, recursed over embedded docs as today. |
| S7 | **Capability mapping unchanged**: `/engine/*` requires `core:write_fields`, same as `/system/*`. The movement gate remains the additional authority on token position. |

## 3. Document shape

```
Document = {
  // ── envelope (engine-structural; typed, ts-rs-exported as today) ──
  id, scope, doc_type, schema_version,
  name: string | null,                      // NEW (S2)
  source, owner, permissions,
  embedded: Record<key, Document[]>,        // children carry the same 3-band shape
  parent_id, created_at, updated_at,

  // ── engine band (S1/S3): present iff doc_type is engine-defined ──
  engine?: <doc_type's generated Engine type>,

  // ── system band: the game system's directory (Nightfox D13/D14) ──
  system: unknown,                          // default {}; engine-opaque, structural-only

  // ── reserved, NOT in M13-0 (S4) ──
  // modules: Record<moduleId, unknown>
}
```

## 4. Engine doc-type registry

Each engine-defined doc_type gets one serde struct in `src/server/src/data/engine/`
(`#[serde(deny_unknown_fields)]`, `#[derive(ts_rs::TS)]`, exported to
`src/types/generated/engine/`). Field sets are the **current shapes verbatim** (source of
truth: the M13-0 inventory of `scene-docs.ts` / `chat-docs.ts` / server reads) minus `name`
(→ envelope). The full body moves for every type except `actor`:

| doc_type | Engine struct contents (today's fields, relocated) |
|---|---|
| `token` | `x, y, w, h, rotation, visual?, actor_id?, overrides?, face?` |
| `scene` | `grid{kind,size,distance?}, background, bounds?, snapToGrid?, vision?, lighting?` |
| `wall` | `seg{x1,y1,x2,y2}, blocksSight?, blocksLight?, blocksMove?` |
| `region` | `shape{kind,points}, behavior, cost, enabled` |
| `light` | `x, y, color, intensity, brightRadius, dimRadius, falloff?, enabled` |
| `drawing` | `shape{kind,points}, stroke{color,width}, fill{color,alpha?}` |
| `template` | `shape{kind,x,y,size,direction}, color` |
| `actor` | `displayName?, visual, size, shape, faction?, conditions, prototype, vision?` — **split type**: all other actor body content is `system` |
| `message` | today's `MessageSystem`, renamed `MessageEngine` (already typed server-side; 100% engine) |
| `world-settings` | `scene{…}, pathfinding{diagonalRule}, animation{speedCellsPerSec,easing}, activeScene` |
| `vision-modes` | `modes: Record<id,{id,name,illuminationFloor,defaultRange,renderHint?}>` |
| `light-gradation` | `bands[]{name,minIllumination}` |
| `chat-settings` | `markdown?, html?, images?, hyperlinks?, emails?, link_previews?` |
| `dice-settings` | `mode, direction` |
| `channel-registry` | `channels: Record<id,{name}>` |
| `faction-registry` | `factions: Record<id,{name,color,stance}>` |
| `condition-registry` | `conditions: Record<id,{name,icon}>` |

**Non-engine doc types** (envelope + `system` only): `item` (its only engine-read field was
`name`), Nightfox's `effect` (D9), `folder`/compendium types when they arrive, and every
community doc_type. Registry membership is a hardcoded server-side match on `doc_type` —
there is no dynamic registration (invariant 6: the server runs no third-party code).

## 5. Server changes

- **Storage**: `Document` gains `name: Option<String>` and `engine: Option<Value>` columns
  /fields beside `system` (persisted as JSON, as `system` is today). `engine` is stored
  post-validation, so stored data always deserializes.
- **Ingress** (`command.rs` apply path): Create with an `engine` body and any `FieldChange`
  under `/engine` triggers deserialization of the post-image `engine` into the doc_type's
  struct; failure rejects the intent. Non-engine doc_type + any `/engine` content ⇒ reject.
  Range clamps that today happen on read (e.g. lighting intensity 0..1) become ingress
  validators where cheap, and remain read-side clamps where not — behavior may only get
  stricter, never looser.
- **Reads**: `scene/mod.rs` pointer walks (`sys_f64`, `resolve_scene`, `sight_walls`,
  `region_field`, `token_vision_floors`, …) refactor to deserialize the stored `engine`
  into the typed struct once per derivation. Existing fail-closed defaults (bounds →
  `DEFAULT_SCENE_BOUNDS_UNITS`, grid size → 100 and > 0, non-finite rejection) are encoded
  as serde defaults + read-side backstop, preserving today's semantics exactly.
- **Move gate**: post-image computation targets `/engine/x`, `/engine/y` (same last-write-
  wins + wholesale-`/engine` bypass-proofing as today's `/system` handling).
- **Redaction** (`permission.rs`): `filter_properties` / `collect_hidden` treat `/engine`
  like `/system` — a whole-band override nulls the band rather than removing a required
  key; `engine` being `Option` redacts to `null`; `/name` redacts to `null`. Boundary
  matching (`/engine/vision` ≠ `/engine/visionmode`) unchanged.
- **Search** (`search.rs`): `index_content` walks `name` ∪ `engine` ∪ `system`;
  `index_content_public` still runs `filter_properties` first (visibility-partitioned
  index invariant holds).
- **Validation** (`validation.rs`): per-block size cap (S6); field-path syntax check
  unchanged.
- **Chat** (`chat/mod.rs`, `chat/settings.rs`): `MessageSystem` → `MessageEngine` read from
  `doc.engine`; settings structs likewise. Server-authored message writes target `engine`.

## 6. Client changes

- **Generated types**: `src/types/generated/engine/*.ts` replaces the hand mirrors. DELETE:
  `scene-docs.ts` body interfaces (`TokenSystem`, `SceneSystem`, `ActorSystem`, …),
  `chat-docs.ts`'s hand-written mirror (its Zod schema may remain as a thin runtime guard
  built to match the generated type), and the render layer's local `WallSystem` /
  `DrawingSystem` / `TemplateSystem` / `RegionSystemLike`. Accessors/resolvers
  (`resolveSceneSettings`, `actor.ts` resolution engine, `resolveTokenBox`, …) re-type
  against generated types and read `doc.engine`.
- **Wire** (`wire.ts`): envelope Zod gains `name: z.string().nullable()` and
  `engine: z.unknown().optional()`; `system: z.unknown()` unchanged (S5).
- **Writers**: scene-tools entity builders, `conditionTarget`, `setNameHidden`
  (→ `/name` override), `setRegionVisibility` (→ `/engine`), sheet engine-band edits emit
  `/engine/...` field changes. `SystemTreeEditor` stays bound to `/system`, which is now
  purely game-system data — the generic tree editor no longer exposes engine internals.
- **Optimistic client**: no semantic change — `engine` participates in OCC pre-images and
  rollback exactly as `system` does (raw-stored-`old` convention applies to both bands).

## 7. Testing & verification

- Full existing suite re-rooted (`/system/x` → `/engine/x`, fixtures reshaped). The sweep
  greps ALL file types for stale `"/system/` literals and `system.<engine-field>` accessors
  (recorded lesson: no narrow include-lists).
- New ingress-rejection battery: wrong field types, unknown engine fields, `engine` on a
  non-engine doc_type, `/engine` field-path writes producing invalid post-images, per-block
  oversize, redaction of `/engine` + `/name` at every tier, FTS non-leak of redacted
  engine leaves.
- Differential guard: the server's typed reads must equal the client resolvers' semantics
  (server-mirrors-client-resolver rule) — the existing vision/movement e2e suites are the
  oracle; all e2e green on the three-OS matrix.

## 8. Out of scope (deferred, recorded)

- **`modules` band (S4)** — reserved key only; design + implementation in a later
  checkpoint (needs its own cap/authz/redaction story per module id).
- **M13f declarative schema registry** — tier-2 validation of `system` stays structural
  until M13f; M13-0 only validates `engine`.
- **Folders/compendium doc types** — arrive later as non-engine types; `name` in the
  envelope already serves them.
- **GM-only secret text on items/effects** (user, 2026-07-15) — requires NO shape change:
  a secret is a `system`-band field redacted per-recipient via
  `property_overrides["/system/<field>"] = "gm_only"` (the mechanism is band-agnostic and
  already canonical in `permission.rs`; the partitioned FTS keeps secrets out of search).
  Deferred piece: the Nightfox authoring surface (sheet field that writes the override
  automatically) — Nightfox spec §12.
- **Dynamic engine-type registration for community placeables** — deliberately impossible
  (invariant 6); a future engine API would be its own design cycle.

## 9. Docs & skills gate (checkpoint completion)

- `ARCHITECTURE.md` §2 invariant 6 rewording: authority over `system` remains structural-
  only; `engine` is typed, server-validated engine territory; the enumerated-geometry
  exception list dissolves into the engine band definition.
- Codebase skills updated: `documents-permissions` (bands, paths, redaction),
  `scene-rendering` (typed reads), `sheets` (`/engine` vs `/system` edit paths), `chat`
  (`MessageEngine`), core skill's ts-rs note (engine types now generated).
- Nightfox spec D15 row gets a pointer to this spec; `docs/PLAN.md` M13-0 entry links it.
