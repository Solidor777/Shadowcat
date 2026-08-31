# M17c — Movement-type tags + terrain exemptions: Plan

**Date:** 2026-08-31
**Spec:** `docs/superpowers/specs/2026-08-31-m17-vision-lighting-movement-design.md` (D5, D9-tags)
**Depends on:** nothing in M17a/b for the core; editors reuse their surfaces. Same branch `m17`.

## Task 1 — Tag fields (server + mirrors)

- `data/engine/token.rs`: `ActorEngine.movement: Vec<String>` (`#[serde(default)]`) and
  `TokenOverrides.movement: Option<Vec<String>>` (wholesale replacement, mirroring `vision`).
- `data/engine/registries.rs`: `Faction.movement: Vec<String>` (`#[serde(default)]`).
- Doc comments state the engine-reserved semantics: `"flying"` / `"incorporeal"` = the mover
  ignores difficult-terrain cost (`terrain_multiplier` reads as 1.0) — and NOTHING else (walls,
  impassable, arrest, the visibility mask all still gate); unknown tags are inert system
  vocabulary. ts-rs + Zod + builders + round-trip/literal-set tests.

## Task 2 — Resolution (client + server, one precedence)

- Client `actor.ts`: `EffectiveActor.movement: string[]` = token-override replacement, else
  dedup(actor.movement ∪ faction.movement). This is the first EffectiveActor field that joins the
  faction RECORD (not just the key) — `resolveTokenActor` already has the store; query the
  `faction-registry` singleton the way `TokenView`/`resolveConditions` do.
- Server: `SceneEcs::token_movement_tags(token) -> BTreeSet<String>` in a new
  `scene/movement_tags.rs` (mod.rs growth discipline), mirroring `token_vision_floors`'s
  linked/instanced/override precedence (embedded-actor branch uncached, same rule). The faction
  union needs the faction registry server-side: hydrate it into the ECS config side-tables exactly
  the way `vision_modes`/`gradation` are (world-config seed already creates the singleton — check
  `set_world_config`'s coverage and extend it; `apply_op` maintenance included).
- Tests: precedence (override replace beats union), faction union, instanced/embedded, dangling
  link ⇒ actor-only… no — dangling link ⇒ empty (mirror `token_vision_floors`'s fallback exactly).

## Task 3 — Threading: one flag, every multiplier site

- Resolved `ignore_terrain: bool` (flying ∨ incorporeal present) computed ONCE per request at the
  two existing seams — `handle_pathfind` and `Room::execute_move` — via
  `token_movement_tags`. New field on `PathInputs` and `MoveGateInputs` (a one-field
  `MoveTraits`/flags struct, room to grow).
- Sites that read it (the full set, verified by grep for `terrain_multiplier`):
  - `pathfinding::astar_leg`'s weighting and `replay_step_costs` (grid router, incl. the continuous
    weighted sub-path that runs through `find`);
  - `SceneEcs::pathfind`'s continuous dispatch predicate — an exempt mover does not force the
    weighted sub-path on `has_terrain_or_impassable` (impassable-only fields still dispatch: the
    exemption is terrain-only);
  - `navmesh::los_smooth`'s `chord_ok` terrain condition and per-span cost;
  - `move_exec::execute_move`'s per-transition pricing and the continuous tail charge.
- Secrecy: the per-requester region-field rules are untouched; an exempt mover's route/cost simply
  ignores terrain they can see AND terrain they can't (the authoritative field springs nothing
  extra — terrain is the one region behavior that never stops a move, only prices it).
- Tests: extend `scene/tests/cost_parity.rs` with an exempt mover on grid AND continuous
  (preview cost == executor cost == unweighted); `move_exec` unified-cost suite gains the exempt
  cases; budget-gate suite proves an exempt mover's decrement uses the exempt cost; the existing
  non-exempt suites stay green untouched.

## Task 4 — Editors

- `module-actors`: movement-tag editor per actor row + create form (chips/multi-select with the
  two reserved tags suggested, free entry allowed — conditions-editor precedent).
- `module-factions`: `movement` field per faction row + the `add()` literal.
- Token override surface: movement override (inherit/replace).
- `ActorSheet`: pass through.
- Tests per module conventions (raw-stored-value OCC pre-images).

## Gates + review

Full gate set as M17a. Reviewers: one movement/parity-focused (router↔executor equivalence,
dispatch-predicate secrecy), one conventions/client-focused.
