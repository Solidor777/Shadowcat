# M17a — Photometric field + carried emitters + light/wall authoring: Plan

**Date:** 2026-08-31
**Spec:** `docs/superpowers/specs/2026-08-31-m17-vision-lighting-movement-design.md` (D1, D2, D9-light, fold-in #2)
**Branch/worktree:** `m17` at `C:/Dev/Shadowcat-m17` (based on `main` @ `df8b1ff2`).

## Task 1 — Photometric composition (server, `scene::lighting`)

- `cell_illumination` → additive superposition: `level = clamp01(Σ contributor levels)`;
  `tint = Σ(levelᵢ × colorᵢ) / Σ(levelᵢ)` (illuminance-weighted RGB mix; zero contributors ⇒
  level 0, tint 0). Per-source `light_illumination` shape unchanged; env contributes
  `env_intensity` where `env_lit`.
- While here, verify the "per-light empty polygon ⇒ unoccluded" semantics in
  `lighting_inputs_from` and document whatever is true at the `LightingInputs` struct.
- Tests (`scene/lighting/tests.rs`): two overlapping dim lights cross the bright threshold; tint
  mix of red+blue; saturation clamps at 1.0; env+placed compose additively; zero contributors ⇒
  dark. Keep every existing fail-closed test green (adjust expected values only where additive
  composition legitimately changes them — `tests-yield-to-correct-code`).

## Task 2 — `LightEmission` struct + `FalloffCurve` enum (server, `data::engine`)

- New `LightEmission` struct in `data/engine/scene.rs`: `{ color: String, intensity: f64,
  bright_radius: f64, dim_radius: f64, falloff: Option<Falloff>, enabled: bool }`,
  `deny_unknown_fields`, ts-rs, doc comments on every field (docs ratchet is live on this tree).
- `Falloff.curve: String` → `curve: FalloffCurve` enum (`#[serde(rename_all = "camelCase")]`,
  `linear | quadratic | none`), ts-rs exported. Update the literal-set test batteries.
- **`LightEngine` restructures to `{ x, y, emission: LightEmission }`** (nested — one emission
  definition, never two structs sharing a shape; no customers, no migration). Wire shape becomes
  `/engine/emission/color` etc. Update the engine round-trip tests and the `scene_lights` parser.
- `ActorEngine.light: Option<LightEmission>` (`#[serde(default)]`) and
  `TokenOverrides.light: Option<LightEmission>` in `data/engine/token.rs`.
- Regenerate ts-rs bindings and commit them with the change; fix the client Zod mirror until the
  drift guard passes.

## Task 3 — Carried emitters in the server field (server, `scene`)

- New module `scene/emitters.rs` (keep `mod.rs` growth minimal): the token→emission resolver
  mirroring `token_vision_floors` precedence exactly — linked actor's `light` replaced wholesale by
  `overrides.light`; instanced token reads the embedded actor (UNCACHED `engine_as`, same rule as
  `token_vision_floors`'s embedded branch); raw token ⇒ `None`. `enabled: false` ⇒ no contribution.
- `SceneEcs::scene_lights` (or a new `scene_emitters` accessor it delegates to) returns standalone
  lights ∪ carried emissions at each token's live position (token `engine.x/y`), one
  `lighting::Light` per emitter. Every consumer of the illumination field gets the union by
  construction — do not add a second read path.
- Doc comment records the D2 decision: the emitter's visibility tier is NOT consulted (physical
  glow; `enabled:false` is the suppress path), mirroring the `gm_only`-wall precedent.
- Tests (`scene/tests/resolution_and_lighting.rs`): carried light lights cells around the token;
  moves when the token moves (two `scene_lights` reads around a position `apply_op`); override
  replace; `enabled:false` suppresses; dangling actor link ⇒ none.

## Task 4 — Fold-in fix: one band resolution (server, `compute_derived`)

- `SceneEcs::compute_derived`'s `vision` arm re-resolves gradation bands that `player_lit_mask`
  already resolved. Resolve once and pass through. Small; prove with the existing derived-channel
  tests staying green.

## Task 5 — Client data model (`@shadowcat/core`)

- `actor.ts`: `EffectiveActor.light: LightEmission | null`; `project()` folds
  override-replaces-actor like `visionModes`. Tests in `actor.test.ts` (linked/instanced/override/
  suppress/dangling).
- `scene-docs.ts`: `buildLightDoc` moves to the nested `{x, y, emission}` shape; its doctest field
  literal updates. Any `LightEngine` consumers updated. Export list additions as needed.
- Wire/Zod mirror for the new/changed types wherever the drift guard reads it.

## Task 6 — Light authoring (scene-tools + render)

- `makeLightTool` in `src/modules/scene-tools/`: GM-only; click places a light at the snapped
  point with documented defaults; re-click/drag repositions; Escape/second tool exits. Dispatches
  the create through the same intent path as `makeWallTool`.
- Light visibility affordance: GMs need to SEE placed lights to edit them. Follow the wall-view
  precedent for how GM-only authoring geometry renders; add a minimal `light-view` (marker +
  radius rings on hover/selected) in `src/client/render/src/` if no existing surface shows lights.
  Check how `WallView` gates visibility and mirror it.
- Light editor: on select (select tool over a light marker), an edit surface for radii / color /
  intensity / falloff curve / enabled / delete. OCC `old` pre-images read the RAW stored value
  (the standing convention).
- Wall editing: select tool over a wall segment opens flag editing (`blocksSight`/`blocksMove`/
  `blocksLight`). Same OCC convention.
- Tests: tool intent dispatch (mirroring `wall-tool.test.ts`), editor updates, GM gating.

## Task 7 — Actor + token light editors

- `src/modules/actors/src/ActorsPanel.svelte`: per-row light emission editor (enabled toggle +
  radii + color + intensity + falloff), and the create-form `ActorEngine` literal gains `light`.
- `ActorSheet.svelte`: same fields through its `setEngine` helper.
- Token override editor: wherever token overrides are surfaced (find it; `TokenOverrides.vision`
  precedent), add the light override (inherit / suppress / replace).
- Tests per module's existing test files.

## Gates (run in the worktree, all must be green before review)

- `pnpm build` (dist/ first — rust-embed compile ordering), `pnpm -r test`,
  `pnpm -r typecheck`, `pnpm lint`.
- `cd src/server && cargo test && cargo clippy -- -D warnings && cargo fmt --check`.
- `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`, `pnpm run test:scripts` if any script or
  skill file changed.
- ts-rs regenerated bindings committed alongside the Rust change.

## Review

Two independent read-only reviewers on the full M17a diff (pre-generated), one focused on
secrecy/secrecy-adjacent paths (the emitter union, additive field, egress), one on the
client/UI + rules-compliance (RULE 15/16, docs gates, conventions). Findings reconciled before
M17a closes.
