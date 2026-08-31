# M17d — Moving light mid-walk: Plan

**Date:** 2026-08-31
**Spec:** `docs/superpowers/specs/2026-08-31-m17-vision-lighting-movement-design.md` (D6)
**Depends on:** M17a (carried emissions), M17b (`light_walls_for` elevation filtering). Same
branch `m17`.

## Task 1 — Wire

- `ws/protocol.rs`: `LightSample { t_ms: u64, polygons: Vec<[f64; 2]>-flat }` mirroring
  `VisionSample`'s shape, and `ServerMsg::MoveStream.mover_light: Option<Vec<LightSample>>`.
  ts-rs export + regeneration.
- Client THREE-edit rule (a new wire scalar is silently stripped otherwise): the `move_stream`
  Zod object in `wire.ts`, the `MoveStream` TS interface in `ws-client.ts`, and the
  snake_case→camelCase mapper in its `"move_stream"` case. Pin with parse + map assertions on both
  the mover and observer paths.

## Task 2 — Server computation ("cost only on request")

- `Room::execute_move`: when the mover's resolved carried emission (M17a's resolver) is `enabled`
  AND the resolved scene is `lighting_enabled && EnvironmentLight`, compute per position sample of
  the already-built `sample_path` walk (same sample list `mover_vision` uses, same
  `MAX_VISION_SAMPLES` cap): the light polygon at that sample position via `bound_for_reach` +
  `vision::visibility_polygon` over `light_walls_for(scene, token_elevation)`, reach =
  `max(bright, dim) × world_units_per_cell`; cap each polygon at `MAX_VISION_POLYGON_VERTS`
  (fail-closed truncation, same rule as vision samples). Hoist the wall set once per move (mirror
  `player_vision_inputs`'s hoist shape).
- Zero-progress moves, GM movers, lightless movers, globalIllumination/lighting-disabled scenes:
  `mover_light: None`, no raycasts.
- The frame registers in the `ActiveStream` registry as today (the Arc'd frame carries the new
  field; the registry is shape-generic).

## Task 3 — Per-recipient admission (server egress)

- `clip_move_stream` branches: mover ⇒ `mover_light` unchanged; plain GM ⇒ unchanged (full
  information, mirrors `cost`); GM see-as ⇒ admission keyed on the TARGET's vision; observer ⇒
  per-sample admission: keep sample `s` iff the disc `(s.pos, dim_reach)` intersects the
  recipient's vision at that instant — committed polygons ∪ the recipient's in-flight own-move
  timelines, resolved through the SAME `move_clip::timeline_polys_at` + `chosen_vision_sample`
  inputs the position clip uses. Disc∩polygon test: center `point_in_poly` ∨ any polygon edge
  within radius (a small `disc_intersects_polys` helper in `move_clip.rs`, pure + unit-tested;
  over-admission is the sanctioned direction, under-admission loses the feature).
- A wholly-inadmissible timeline ⇒ `mover_light: None` for that recipient (the position stream
  itself still clips exactly as today — unchanged).
- Re-emit pass: `concurrent_streams` re-clip covers the new field automatically if
  `clip_move_stream` handles it — verify with a test, don't assume.
- Tests (`ws/conn` + `ws/move_clip`): admitted mid-walk reveal; cross-map recipient gets `None`;
  sample-level filtering; see-as narrowing; mover/GM branches; secrecy regression suite stays
  green.

## Task 4 — Client lighting sweep

- `RenderEngine`: `lightSweeps: Map<tokenId, {samples, elapsed, durationMs}>` beside
  `visionSweeps`; `animateSamples` gains the `moverLight` parameter (through
  `WorldSession.onMoveStream` → `sceneInteraction.animateSamples` → engine — update the bridge
  signature and its fakes).
- While a light sweep is in flight, the lighting overlay = last committed `Lighting` frame ∪ the
  current light sample's polygon (cross-fade between consecutive samples via the `fog-blend`
  factor helper + the same rasterized-texture blend path `setVisibilityBlend` uses — the lighting
  layer's paint path may need a `setLightingBlend` analog; extract whatever is GL-bound behind a
  pure seam like `fog-blend` for unit tests).
- The chosen-sample rule MUST match the server's `chosen_vision_sample` — extend the shared
  fixture `__fixtures__/chosen-vision-sample.json` coverage to the light timeline rather than
  writing a second rule.
- On sweep completion: revert to the derived channel (the post-commit rebroadcast carries the
  light's final position). Concurrent light sweeps union, mirroring `visionSweeps`.
- Tests: sweep advance/union/revert, gap-free handoff to the derived frame, observer with no
  `mover_light` unchanged.

## Task 5 — E2E

- Playwright: carried torch on a player's move lights a corridor for an observing player mid-walk
  (assert the lighting overlay changes before move end); a cross-map observer's lighting never
  changes.

## Gates + review

Full gate set as M17a. Reviewers: one secrecy-focused (admission bounds, registry lifetime, the
invariant-11 residual exactly as spec'd), one client/sweep-focused.
