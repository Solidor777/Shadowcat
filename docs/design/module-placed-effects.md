# Module-placed effects

A module or system lets a player place an area with effects, for example a spell whose area the player chooses. The effectful placement is authorised on the server by the module's manifest declaration. It is never authorised by the player's client alone.

## The seam

The seam is a declaration in the module manifest plus one server intent that consumes it.

1. **Declaration.** The module manifest (`module.json`) carries a `placements` list. Each entry (`PlacementDecl`) names the effect set, the limits on the area, and who may place it.
2. **Intent.** The client sends `ClientMsg::PlaceEffect` carrying only the module id, the declaration id, the scene, the chosen area (a `RegionShape`), and optionally a caster token. The frame has no field for effects, behavior or triggers, so a client cannot choose them.
3. **Authority.** `Room::place_effect` resolves the declaration from the world's enabled, engine-compatible modules (the same `ModuleScanCache` and `engine_compat_ok` gate the capability requirements use). It decides who may place it from the declaration's `who`, clamps the area to the declaration's limits, builds the region from manifest data, validates it with `RegionEngine::validate`, and commits one `Operation::Create` under `WriteOrigin::ModulePlacement`. The same origin carries the later `Operation::Delete` that removes an expired region (see Expiry).
4. **Write origin.** `WriteOrigin::ModulePlacement` is server-authored and waives the per-op capability floor, like `DoorToggle`. It is never derivable from a wire frame. A client `Create` of a region still needs the ordinary `core:create` grant for `region`, which players do not hold by default, so a player client alone cannot create an effectful region.

No module code runs on this path. A module's wasm validator (`sandbox`) still judges the committed document through the normal `apply_intent` pass and can refuse it.

## Declaration shape

```json
"placements": [
  {
    "id": "fireball",
    "label": "Fireball",
    "who": "caster_owner",
    "shapes": ["circle"],
    "maxExtent": 12,
    "behavior": "terrain",
    "cost": 1,
    "triggers": [
      { "on": "enter", "effect": { "type": "condition_add", "condition": "burning" } }
    ],
    "durationSecs": 60,
    "maxRangeCells": 6
  }
]
```

- `who` is required: `gm` (only a GM), `player` (any player in the world), or `caster_owner` (the request names a caster token on the scene that the sender effectively owns). A GM may always place, whatever `who` says.
- `shapes` lists the allowed `RegionShape` kinds (`circle`, `rect`, `polygon`).
- `maxExtent` is the largest bounding-box side of the area, in scene units.
- `behavior`, `cost`, `triggers` and optional `elevation` are copied into the region's engine body. The client cannot change them.
- `durationSecs` (optional, 1 to `MAX_PLACEMENT_DURATION_SECS`, 30 days) and `maxRangeCells` (optional, finite and positive) are the two declared limits, described below.
- A declaration that fails parsing or `RegionEngine::validate` against a probe shape is dropped with a warning at module scan; the module itself still loads (the same fail-open discovery `systemDefaults` uses).

## Expiry

A declaration with `durationSecs` produces regions that the server removes.

- **Stamp.** `Room::place_declared` writes `system.placement.expires_at` (wall-clock milliseconds, the server's commit time plus the duration; `PlacementDecl::expires_at`) into the region document. A declaration without a duration writes no stamp and the region persists until a GM deletes it.
- **Removal.** `Room::expire_due_placements` finds the regions whose stamp is at or before now (`SceneEcs::due_placement_regions`, read from the scene entities) and deletes them as one batch under `WriteOrigin::ModulePlacement`, authored as the world's seed author (`world_seed::seed_author`, a GM member). The delete is an ordinary committed command, so every client sees the region removed through the normal event stream.
- **Trigger.** `spawn_placement_expiry_sweep` runs one background task per server, started by `http::serve_with`, which asks every live room to run that removal once a second (`PLACEMENT_EXPIRY_PERIOD`) and ends with the server's shutdown request. A region can therefore outlive its stamp by up to one period.
- **Restart.** The stamp is part of the persisted region document and a room is hydrated from the persisted documents, so nothing is lost across a restart: a region that expired while the server was down is removed on the first sweep after its world's room is created, and one that has not expired keeps its original deadline. A world with no live room runs no region effects, so a stale region there does nothing until a room exists.
- **Why not the combat clock.** Combat effect durations (`combat::effects`) count turns and rounds of a running combat and live on `effect` documents. A placed area is a `region`, needs no combat, and is meant to last wall-clock time, so a wall-clock stamp is the fit. No existing server timer removes documents on a wall-clock deadline (the trash and session sweeps remove files and rows, not documents), so this is the first.

## Caster range

A declaration with `maxRangeCells` limits how far the area may be from the caster token.

- `Room::check_caster_range` resolves the named caster on the request's scene and reads its position. The measure is `PlacementDecl::within_range`: `euclidean_span_cells` (the shared scene-point span in cells the move executor prices continuous spans with) between the caster's position and the centre of the area's bounding box, at the scene's `GridShape::world_units_per_cell`, the same cells-to-scene-units conversion the door-reach check uses. The limit is inclusive.
- A non-GM must name a caster they effectively own on the scene, whatever `who` says, so omitting the caster cannot dodge the limit. A GM who names a caster is measured too; a GM who names none is not limited.
- A refusal is `PlaceEffectError::OutOfRange` (a missing or foreign caster is `CasterNotOwned`). A non-GM gets "placement rejected"; a GM gets "the area is out of range of the caster".

## Rejections

`ServerMsg::PlaceEffectError { request_id, message }` goes to the requesting connection only. A non-GM gets one wording for every refusal ("placement rejected"), so a refusal does not disclose whether a module, declaration or token exists. A GM gets the specific reason.

## Later programmable path

`Room::place_effect` takes the resolved declaration and the validated area as its inputs, and builds the document from them. A later programmable path (a guest that computes the effect set) feeds the same function with a computed declaration. That path is not built.
