# M25 — Multi-level maps + portals — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.4 the seam this milestone
> owns; §9 D4). Goal: one scene can hold several floors (a tower, a dungeon with a
> basement), each with its own map image and geometry, tokens live on the floor their
> elevation puts them on, and a region can teleport a token — across floors or across scenes.

## 1. Data (`data::engine::scene`, `data::engine::geometry`)

```rust
SceneEngine.levels: Vec<SceneLevel>          // appended, #[serde(default)]; empty = one implicit ground level
SceneLevel { id: String /* non-empty, ≤ 64, unique within the scene */, name: String,
             bottom: f64, top: f64 /* finite, bottom < top */, background: Option<String> /* asset id */ }

/// `WallElevation` is RENAMED `ElevationBand` (one type for every banded geometry; every
/// `WallElevation` citation moves with it — churn accepted); `Option<ElevationBand>` is added to:
RegionEngine.elevation, DrawingEngine.elevation, TemplateEngine.elevation (an AoE template is placed
on a floor); LightEngine and TokenEngine keep `elevation: Option<f64>` (a point)
```

- `SceneEngine::validate`: ≤ 32 levels, unique ids, non-overlapping `[bottom, top)` bands,
  finite; a `background` id non-empty when present.
- `scene::elevation::level_of(levels: &[SceneLevel], elevation: f64) -> Option<&SceneLevel>`:
  the level whose `[bottom, top)` contains `elevation`; if none contains it, the highest level
  whose `bottom ≤ elevation` (a token on a roof is on the top floor); below every level ⇒ the
  lowest level; `levels` empty ⇒ `None`. This slice-taking signature IS the one shared
  symbol the master §2.4 names; callers holding a scene pass `&scene.levels`. Mirrored by
  `levelOf(levels: SceneLevel[], elevation): SceneLevel | null` in `@shadowcat/core`
  (`src/client/core/src/levels.ts`), pinned by a shared JSON conformance corpus at
  `src/client/core/src/__fixtures__/levels-conformance.json`, read by a Rust test through a
  relative path and by a Vitest test — exactly the formula corpus's shape
  (`src/client/formula/src/__fixtures__/conformance.json`).
- `wall_occludes(wall, e)` (exists, vision) is joined by ONE sibling predicate
  `band_contains(band: Option<&ElevationBand>, e: f64) -> bool` that `wall_occludes`,
  the movement gate, `regions::rasterize`'s per-level selection and the drawing/region
  render filters ALL call — the never-fork pin.

## 2. Server semantics

- **Movement.** The sole per-cell traversal decision is `scene::move_exec::execute_move`/
  `gate_walk` (`Room::publish` runs no wall machinery of its own — it refuses non-GM position
  changes outright and routes moves through the executor); it, `pathfinding::cell_enterable`
  and the navmesh build consult `band_contains(wall.elevation, mover_elevation)` through
  `WallEngine::blocks_move` — a floor-2 wall no longer blocks a floor-1 mover (today
  `blocks_move` ignores elevation; the `WallEngine::elevation` doc comment's "never consulted
  by the movement gate" sentence is deleted, since it becomes false).
  `SceneEcs::region_field(scene, viewer)` gains a THIRD parameter, `elevation` — the existing
  `viewer` secrecy filter (`engine_tier_visible`) and the new band filter are independent
  conjuncts; the composed field is built from regions whose band contains the mover. The
  navmesh cache key `(scene, footprint_radius_cells)` gains the level id
  (`navmesh_for(scene, footprint_radius_cells, level)`); `trigger_regions` likewise selects
  by band.
- **Vision + secrecy.** Levels are opaque planes: two entities on different levels never see
  each other. `sight_sources`/the raycaster are already elevation-banded on walls, so vision
  POLYGONS need no change; the `"vision"` payload's polygons carry the source token's `level`
  (through `level_of`) so the client cuts fog holes only into the viewed level's fog. Resting
  token positions ride the position `Event` to every scene reader and are hidden CLIENT-SIDE
  by the level scope (§3) — the same sent-then-hidden model resting tokens already follow for
  fog (invariant 11; there is no server-side fog stripping of resting tokens today, and this
  milestone adds none). In-flight moves keep their existing server-side per-recipient clip
  (`move_clip`), which gains the level conjunct on the SAME predicate it already applies —
  a mover on another level is clipped like a mover out of sight. Lighting: a light contributes
  only to the level `level_of(light.elevation)` resolves to (the lit mask is built per level;
  `player_lit_mask(scene)` → `(scene, level)`).
- **Explored fog per level.** `explored_fog` gains `level_id TEXT NOT NULL DEFAULT ''` in its
  primary key (edit `0001_init.sql` in place); `Repository::get_explored(scene, level, user)`
  / `set_explored`; `enrich_vision_explored` accumulates each polygon into the level of its
  source token and emits the explored blob of the recipient's VIEWED level (a new
  `SceneSubscribe.level: Option<String>` field; `None` ⇒ implicit ground).
- **Derived channels.** `"footprints"`, `"vision"`, `"lighting"` payload entries carry
  `level: Option<String>` so the client scopes by level without re-deriving it; `"audibility"`
  (M23b) needs nothing — its raycast is elevation-banded and the listener/emitter elevation
  pair already separates floors (M25's integration task adds a test proving a floor-2 emitter
  is occluded for a floor-1 listener only when a banded wall/floor rule says so — the audio
  spec treats floors as opaque: emitters on another level get `through_wall_gain`).
- **Portals — `TriggerEffect::Teleport`** (appended variant):

  ```rust
  Teleport { target: PortalTarget }
  PortalTarget { scene: Option<Uuid> /* None = same scene */, x: f64, y: f64,
                 elevation: Option<f64>, vfx: Option<String> /* asset played at both ends */ }
  ```

  Validation: finite, `MAX_GATE_WALK_COORD`-bounded coordinates. `validate_engine_tree` is
  pure (no repository access), so the target scene's existence is checked at FIRE time: a
  missing target scene ⇒ GM-only notice, no move.
  Fires from `Room::fire_region_triggers` on `Enter` (movement AND placement sites, as every
  effect). Application, in the same effect-application helper: same-scene ⇒ a server-authored
  Update of `/engine/x`, `/engine/y` (+ `/engine/elevation` when given); cross-scene ⇒ the
  server-authored `Operation::Move` (M15b's reparenting op) to the target scene followed by
  the position Update in one batch. Origin: a NEW `WriteOrigin::Trigger` replacing the
  `CombatTransition` origin the M18 trigger effects borrow (every trigger effect moves to it;
  `skips_capability_gates` adds it beside `CombatTransition | ConfigSeed | TemplateMerge`).
  **`apply_intent`'s `Operation::Move` arm does NOT call that predicate today — it compares
  the origin literally against `CombatTransition` and otherwise demands `WorldRole::Gm` with
  full access;** this milestone converts that literal to `!origin.skips_capability_gates()`
  (the shape every other gate site in `apply_intent` already uses), or a player-owned token
  could never be teleported across scenes. After a teleport the
  destination cells fire `Enter` effects EXCEPT `Teleport` (one hop per move; a chained portal
  is refused with a GM-only notice) — no loops. A `vfx` id is broadcast as two
  `ServerMsg::Vfx` frames (source + destination) through M24's handler path in the
  integration task; before M24 merges the field is validated and carried, not played.
  Combat: a `combat` document is bound to ONE `scene_id`, and the movement-budget gate
  resolves the combatant from the token's CURRENT scene (`active_combat_for_scene` /
  `combatant_for_token`). A combatant teleported off the combat's scene therefore keeps its
  combatant record and turn but moves unbudgeted on the destination scene until it returns or
  the combat ends — intended (a portal is a legitimate escape), and made visible: the
  teleport posts a GM-only notice naming the combatant when an active combat is on the source
  scene. The budget decrement for the walk that entered the portal already happened.

## 3. Client

- **Scoping.** `scene-scope.ts`'s `sceneScopedDocs(store, docType, viewedSceneId)` gains a
  fourth argument `viewedLevel: () => string | null` (a getter, like `viewedSceneId`); every
  view — the four near-identical shape views `WallView`, `RegionView`, `DrawingView`,
  `TemplateView`, plus `LightView`, `TokenView`, and `VfxView` after M24 merges — filters by
  the `bandContains`/`levelOf` mirrors in `@shadowcat/core` inside that one helper; the views
  never test elevation themselves. Background: `SceneLevel.background ?? SceneEngine.background`.
  GM-only "ghost other levels" toggle draws other levels' tokens at 30 % alpha (a `TokenFx`
  `desaturate` + alpha — no new mechanism).
- **Viewed level.** `AppContext.viewedLevel: string | null`, `setViewedLevel(id | null)`;
  default for a player = `levelOf` of their primary token (updated when that token's
  elevation changes — follows the token through a portal); for a GM = the last chosen level
  per scene (persisted in `ui_state.worlds[id].viewedLevel`). `WorldSession` re-subscribes the
  scene with the new `level` on change. `viewedLevel` is `null` for a level-less scene.
- **UI.** `src/modules/stage/`'s chrome (or `scene-tools`' rail — whichever hosts the existing
  scene affordances) gains `LevelSwitcher.svelte` (segmented control of level names, hidden
  when the scene has no levels, touch-sized). The scene sheet/scene-browser edit form gains a
  `LevelsEditor.svelte` (list; band numbers; background pick via `ctx.pickAsset`; add/remove;
  the whole-array write pattern). Scene-tools stamp the viewed level's band onto NEW walls/
  regions/drawings (`elevation: { bottom, top }`) and the level's `bottom` onto placed
  tokens/lights, so authoring on a floor lands on that floor without a numeric field.
  The region tool's trigger editor gains the `Teleport` effect: target scene picker
  (`ctx.searchDocuments({ docTypes: ["scene"] })`), x/y with a "pick on stage" button that
  temporarily switches the viewed scene and captures one click, elevation, VFX pick.
- **Movement preview.** The pathfinder preview already comes from the server (`Pathfind`);
  the request carries no elevation today — it gains none: the server reads the mover's stored
  elevation (never the client's claim).

## 4. Tests

- Server: `level_of` corpus; `SceneEngine::validate` (overlap, count, ids); `band_contains`
  parity test — mutate the movement gate's call and the vision test fails (one predicate);
  walls per level in `execute_move`/`pathfind`/`gate_walk` (a floor-2 wall lets a floor-1
  mover through, blocks a floor-2 mover); `region_field(scene, elevation)` selects by band;
  navmesh cache keyed by level; `move_clip` clips a mover on another level; the `"vision"`
  payload tags polygons with their level; lit mask per level; `apply_intent`'s `Move` arm
  accepts `WriteOrigin::Trigger` for a player-owned token; explored per level round-trip +
  accumulation into the source token's level; `Teleport` same-scene, cross-scene (`Move` +
  position, both in one seq), missing scene ⇒ notice only, chained portal refused, destination
  `Enter` effects fire minus teleport, `WriteOrigin::Trigger` on every trigger write.
- Client: `levelOf` corpus; `sceneScopedDocs` level filter for every view (a token on
  another level is absent from the player's stage and ghosted on the GM's); `viewedLevel` default and
  follow-through-portal; `LevelSwitcher`/`LevelsEditor` component tests; scene-tools stamping.
- e2e `levels.spec.ts` (written here; dispatcher-run): GM authors two levels with distinct
  backgrounds; places a player token on level 1 and an NPC on level 2 → the player's stage
  shows `data-level="l1"` and `data-token-count="1"`; the GM switches to level 2 → sees the
  NPC; GM draws a teleport region on level 1 targeting level 2; the player walks into it →
  the player's `data-level` becomes `"l2"` and the token is visible to the GM on level 2.

## 5. Docs + skills

- `docs/site/modules/stage.md` (levels), `scene-tools.md` (stamping, teleport trigger);
  `protocol.md` (`scene_subscribe.level`, channel `level` fields); ARCHITECTURE §4's
  multi-level row rewritten as built; the `WallEngine::elevation` doc and the region skill's
  "never consulted by movement" statements corrected.
- Skills: `scene-rendering` (levels, `level_of`, `band_contains`, explored per level,
  `WriteOrigin::Trigger`, portals), `documents-permissions` (the level conjunct in the egress
  predicate). No new skill (master §6).
- `docs/HISTORY.md` M25 entry.
