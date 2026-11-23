# Module-placed effects

A module or system lets a player place an area with effects, for example a spell whose area the player chooses. The effectful placement is authorised on the server by the module's manifest declaration. It is never authorised by the player's client alone.

## The seam

The seam is a declaration in the module manifest plus one server intent that consumes it.

1. **Declaration.** The module manifest (`module.json`) carries a `placements` list. Each entry (`PlacementDecl`) names the effect set, the limits on the area, and who may place it.
2. **Intent.** The client sends `ClientMsg::PlaceEffect` carrying only the module id, the declaration id, the scene, the chosen area (a `RegionShape`), and optionally a caster token. The frame has no field for effects, behavior or triggers, so a client cannot choose them.
3. **Authority.** `Room::place_effect` resolves the declaration from the world's enabled, engine-compatible modules (the same `ModuleScanCache` and `engine_compat_ok` gate the capability requirements use). It decides who may place it from the declaration's `who`, clamps the area to the declaration's limits, builds the region from manifest data, validates it with `RegionEngine::validate`, and commits one `Operation::Create` under `WriteOrigin::ModulePlacement`.
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
    ]
  }
]
```

- `who` is required: `gm` (only a GM), `player` (any player in the world), or `caster_owner` (the request names a caster token on the scene that the sender effectively owns). A GM may always place, whatever `who` says.
- `shapes` lists the allowed `RegionShape` kinds (`circle`, `rect`, `polygon`).
- `maxExtent` is the largest bounding-box side of the area, in scene units.
- `behavior`, `cost`, `triggers` and optional `elevation` are copied into the region's engine body. The client cannot change them.
- A declaration that fails parsing or `RegionEngine::validate` against a probe shape is dropped with a warning at module scan; the module itself still loads (the same fail-open discovery `systemDefaults` uses).

## Rejections

`ServerMsg::PlaceEffectError { request_id, message }` goes to the requesting connection only. A non-GM gets one wording for every refusal ("placement rejected"), so a refusal does not disclose whether a module, declaration or token exists. A GM gets the specific reason.

## Later programmable path

`Room::place_effect` takes the resolved declaration and the validated area as its inputs, and builds the document from them. A later programmable path (a guest that computes the effect set) feeds the same function with a computed declaration. That path is not built.

## Not built here

- Expiry of a placed region. A placed region persists until a GM deletes it.
- Range checks from the caster to the area.
