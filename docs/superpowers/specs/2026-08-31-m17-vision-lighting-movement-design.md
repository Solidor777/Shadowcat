# M17 — Vision, Lighting & Movement Completion: Design Spec

**Date:** 2026-08-31
**Status:** Brainstorm output; decisions below are locked per the campaign directive
(best-long-term-shape test applied to each fork), subject to user review on commit.
**Supersedes/expands:** the Phase-2 deferrals in `2026-06-22-m9-walls-vision-fog-design.md` §Out
(photometric / illumination coupling, darkvision / tremorsense / height), the M10g deferral in
`2026-07-01-m10g-regions-design.md` §10.2 (per-actor/faction movement exemptions), and the parked
residual in `2026-08-27-move-stream-live-clip-design.md` §1 case 2 (third-party moving light source).

## 1. Goal

Complete the vision/lighting/movement foundation begun in M10e. Four deliverables:

1. **Photometric lighting** — one additive illumination field composed from *emitter components*
   (standalone light documents + token/actor-carried emissions + the environment ambient class),
   replacing the special-cased flat/edge-projected environment-light model and the max-of-sources
   composition. Plus the senses expansion: **tremorsense** and **height/elevation** (darkvision
   already exists as an illumination-floor mode and stays).
2. **Per-actor/faction movement exemptions** — movement-type tags on actors (flying/incorporeal
   ignore difficult-terrain cost), resolved identically at the router and the executor.
3. **Moving light source mid-walk** — a mover carrying a light reveals per sample of the move, not
   at its stop; the per-sample light timeline is computed only when a carried light exists
   ("cost only on request").
4. **Lighting authoring** — the completion half of "completion": light placement/editing UI (the
   `light` doc_type currently has NO creation path), wall flag + elevation editing, and the actor /
   faction / token editors for every field this milestone adds.

**M14 dependency check:** nothing in this design keys on the turn owner. The PLAN.md dependency line
("M14 for anything keyed to the turn owner") is not exercised; no M14 surface is touched.

## 2. Constraints inherited (cited inline)

- **Secrecy gate, fail-closed** — the vision mask and its derived channels only ever under-reveal on
  malformed input; fog remains the rendered secrecy gate (`fog-is-the-secrecy-gate-fail-closed`).
- **UX outranks data secrecy (ARCHITECTURE §2 invariant 11)** — send-then-hide is acceptable where
  secrecy and fidelity conflict; it never governs PII or remote-compromise. The mid-walk light
  timeline and the tremorsense perception list are designed under this rule, with the narrower
  per-recipient option kept wherever its cost is comparable.
- **Server-authoritative geometry** — vision, lighting and movement decisions are computed on the
  server; the client renders what it is sent and never computes its own visibility.
- **Never fork a decision across two paths** — router and executor, egress mask and movement gate,
  client preview and server resolution must read one shared symbol.
- **Three-band document shape (invariant 6)** — new engine fields land in the typed `engine` band
  (`deny_unknown_fields`, ts-rs-exported, Zod-mirrored); no `system`-band pointer walks.
- **No data migrations pre-customers** — engine-struct changes edit the structs in place; the SQL
  baseline is untouched (no schema change is needed: every addition is document JSON).
- **File-size limits** — `scene/mod.rs` (3.4k lines) must not approach the 5,000-line soft limit;
  new resolution logic lands in sibling modules (`scene/emitters.rs`, `scene/senses.rs`, …), not in
  `mod.rs`.

## 3. Current state (measured, not assumed)

- Placed `light` docs exist with `x/y/color/intensity/brightRadius/dimRadius/falloff/enabled`;
  falloff curves (linear/quadratic/none) are fully wired server-side, but `Falloff.curve` is a
  stringly-typed struct and **no UI anywhere creates or edits a light** (`buildLightDoc` has no
  production caller).
- Environment light is edge-projected from the scene boundary, `blocksLight`-occludable,
  flat-intensity where admitted. Composition across sources is **max**, with a single dominant
  tint (no additive mixing, no color blending).
- Vision modes are data-driven (`vision-modes` config doc): `{id, name, illuminationFloor,
  defaultRange, renderHint}`; seeds are `normal` (floor `dim`) and `darkvision` (floor `dark`,
  range 12, hint `desaturate`). The mask is `LOS ∩ (lit ∨ lowered-floor-in-range)` per user.
- No token/actor light emission, no elevation/height anywhere, no tremorsense, no movement-type
  tags; walls are authored with all three block flags hardcoded `true` and cannot be edited after
  placement.
- The move-stream live clip closes the observer's-own-move case; the third-party moving-light case
  is the parked residual this milestone takes.
- TODO.md fold-in (actionable now, this subsystem): `player_lit_mask` and
  `gather_vision_sources_in_scene` each hand-roll the observer-tier role lookup that
  `effective_role` owns — a forked decision that widens vision on divergence. Fixed here.

## 4. D1 — Photometric composition (the illumination field)

`cell_illumination` changes from max-compose to **additive superposition with saturation**:

```
level(cell) = clamp01( Σ_sources contribution(source, cell) )
contribution(placed/carried light) = intensity × falloff(t)   — unchanged per-source shape
contribution(environment)          = env_intensity where edge-projection admits the cell
tint(cell) = Σ(levelᵢ × colorᵢ) / Σ(levelᵢ)                   — illuminance-weighted mix
```

- **Superposition is the photometric property**: two dim lights brighten their overlap; a torch in
  dim daylight reads brighter than either alone. Bands still threshold the composed `[0,1]` value,
  so the gradation model, the wire payload (`[i, j, band, tint, hint]` cells) and the client
  renderer are unchanged in shape — only the computed values change.
- **Environment light becomes the ambient emitter class inside the same field**, keeping its
  edge-projection and `blocksLight` occlusion exactly (that occlusion is a load-bearing secrecy
  input — it is what keeps interiors dark under daylight). What dies is its status as a
  separately-composed special case: it is one more additive contributor. ("Replacing the
  flat/edge-projected environment light model" is read as replacing the *model boundary*, not
  deleting ambient occlusion; a distance-falloff ambient was considered and rejected — no plan item
  consumes it and it multiplies compute per scene.)
- `Falloff.curve` becomes a real serde/ts-rs enum (`linear | quadratic | none`) — the stringly-typed
  struct is a v1 artifact with no UI; this is the milestone that gives it one.
- Fail-closed directions are preserved per source (non-finite/disabled ⇒ contributes 0; empty env
  polygon set ⇒ no env contribution). Additive composition cannot darken any cell relative to the
  pre-change model for the same content, so no existing scene loses visibility.

## 5. D2 — Carried lights (illumination coupling)

Light emission couples to tokens/actors through the same machinery vision modes use:

- **`LightEmission`** — one shared struct `{ color, intensity, brightRadius, dimRadius, falloff?,
  enabled }` (radii in cells, the `LightEngine` value fields minus position), used by both
  `ActorEngine.light: Option<LightEmission>` and `TokenOverrides.light: Option<LightEmission>`.
  Absent override key ⇒ inherit; `enabled: false` ⇒ this token's emission is off (the suppress
  path). `LightEngine` keeps its own `x/y/enabled` for standalone lights.
- **`EffectiveActor.light`** joins the resolution (`resolveTokenActor` client-side; the
  `token_vision_floors`-mirrored server resolver), wholesale replacement on override, exactly like
  `vision`.
- **Server union:** the illumination field's light set = standalone `light` docs ∪ carried
  emissions resolved at each token's live ECS position. Carried lights raycast against the same
  `light_walls` set with the same `bound_for_reach` reach rule. A carried light participates in
  every consumer of the field (lit mask, movement gate, env composition) with no second code path.
- **Visibility of the emitter is not consulted.** A fogged or permission-hidden token's carried
  light still illuminates — physically the glow precedes the bearer, and the GM authored the
  emission (suppress it with `enabled: false`). This mirrors the standing rule that a `gm_only`
  wall still blocks sight. The converse is NOT true for tremorsense (§6), which lists readable
  tokens only.
- M18's "light emitters as token components" builds on exactly this: the emission struct is the
  component payload; M18 generalizes it to aura/sound/VFX kinds.

## 6. D3 — Senses: tremorsense + height

### 6.1 Vision-mode descriptor v2

`VisionMode` gains two fields, defaulting so every existing mode is unchanged:

```jsonc
{
  "id", "name", "defaultRange": <cells, 0 = unlimited>,
  "illuminationFloor": "<band>",        // terrain senses
  "perceives": "terrain" | "creatures", // default "terrain"
  "requiresLos": true,                  // default true
  "renderHint": null
}
```

- **Terrain senses** (normal, darkvision, blindsight-as-floor-dark) behave exactly as today:
  `LOS? ∩ illumination ≥ floor`, range-limited.
- **Creature senses** (tremorsense): perceives *tokens*, not terrain. A recipient perceives a token
  T iff some vision source S carries a creature sense whose range covers dist(S, T), **and both S
  and T are grounded** (elevation 0 — this is the whole "same ground" rule; a flying token is
  immune the moment it leaves the ground, no movement-tag coupling). `requiresLos: false` ignores
  walls; illumination is irrelevant.
- Seed addition: `tremorsense { perceives: "creatures", requiresLos: false, defaultRange: 12 }`.
  Truesight/blindsight slot in as data later (blindsight ≈ darkvision with a range); no illusion or
  invisibility mechanic exists for truesight to pierce yet, so no seed.

### 6.2 The `perceived` channel (wire)

The masked `vision` payload gains `perceived: [{ scene, tokens: [uuid] }]` — the ids of tokens the
recipient perceives *only* through a creature sense (already-visible tokens are not restated).
Properties:

- **Per-recipient, server-computed** in the same pass as the lit mask (GM `mode:"all"` carries
  none; see-as computes as the target, inheriting the correct set).
- **READ-gated:** `perceived` ⊆ tokens the recipient's document stream already delivers. A
  permission-hidden token is never named — creature senses pierce fog, not the READ gate.
- **Never accumulated into explored fog** (creatures, not terrain; nothing to remember).
- Client render: a perceived token renders through fog/darkness (the send-then-hide direction
  invariant 11 already sanctions — resting tokens ride the document stream to every scene reader
  today). It does not punch fog holes for terrain; the token alone is raised.

### 6.3 Height / elevation (2.5D, single-level)

- **`TokenEngine.elevation: f64`** (default 0) — token state, not actor state; flying is a tag
  (§7), altitude is per-token. **`LightEngine.elevation: f64`** (default 0); a carried emission's
  elevation is its token's.
- **`WallEngine.elevation: { bottom?: f64, top?: f64 }`** — absent wall ⇒ `{−∞, +∞}` (blocks every
  elevation, exactly today's behavior); an absent end is unbounded.
- **Occlusion rule (source-elevation band test):** a wall occludes a sight/light source at
  elevation `e` iff `bottom ≤ e ≤ top`. Ground viewer (0) vs a normal `{0,…}` wall: blocked (today's
  behavior). Viewer above the wall top: sees over. Viewer below the wall bottom (bridge overhead):
  sees under. The same rule filters `light_walls` per light. **Environment ambient keeps its
  current all-elevations occlusion** — walls always shadow sky-light, or daylight would flood
  interiors.
- This is deliberately the simple model: blocking is target-distance-independent, so the
  star-shaped visibility-polygon pipeline (raycast → rasterize → client fog mask → explored
  accumulation) is untouched; elevation only *filters the occluder set per source*. The physically
  exact alternative (shadow strips with over-the-wall re-emergence) breaks star-shapedness and buys
  fidelity no plan item consumes; multi-level maps (Phase 3) revisit elevation with real floor
  data. The model statement above is the definition, so there is no over/under-reveal delta to
  audit — but the *direction* of any future refinement must stay under-reveal.
- Sight/illumination *ranges* stay 2D (horizontal); elevation affects occlusion and tremorsense
  grounding only.
- Client: an elevation badge on tokens with elevation ≠ 0 (the badge seam already exists for
  condition markers); authoring on the token (§9).

## 7. D5 — Movement-type tags + terrain exemptions

- **`movement: Vec<String>`** on `ActorEngine` (default `[]`) and `TokenOverrides.movement`
  (wholesale replacement, same shape as `vision`); **`Faction.movement: Vec<String>`** unioned in
  at resolution — faction-level traits apply to every member (the first faction→EffectiveActor
  property flow; the faction record join already exists for color/stance consumers).
  `EffectiveActor.movement` = dedup(actor ∪ faction) or the token-override replacement.
- **Engine-reserved tags:** `flying` and `incorporeal`, each meaning **ignores difficult-terrain
  cost** (the `terrain_multiplier` reads as 1.0). Nothing else: impassable regions still block,
  arrest regions still stop, walls still gate, the visibility mask still gates. Unknown tags are
  carried as inert data (system vocabulary space, same posture as conditions).
- **One resolver, both engines, both paths:** `SceneEcs` gains a
  `token_movement_tags`-style resolver (mirroring `token_vision_floors`' linked/instanced/override
  precedence, plus the faction union) consumed at the two existing seams — `handle_pathfind` and
  `Room::execute_move` — and threaded into `PathInputs` / `MoveGateInputs` as resolved flags. Every
  terrain-multiplier site reads the flag: grid A* weighting + the shared step-cost replay, the
  continuous dispatch predicate (an exempt mover's scene terrain does not force the weighted
  sub-path), `los_smooth`'s chord rule and per-span cost, and the executor's per-transition +
  continuous tail pricing. The cost-parity suites extend to exempt movers so preview and execution
  cannot diverge.
- The combat movement-budget gate consumes `MoveOutcome.cost` unchanged — an exempt mover simply
  spends less, identically in preview and execution.

## 8. D6 — Moving light mid-walk

Builds on D2 (a carried light exists to move) and the live-clip machinery (per-sample timelines,
per-recipient egress clip, client sweeps).

- **Wire:** `MoveStream` gains `mover_light: Option<Vec<LightSamplePt>>`,
  `LightSamplePt { t_ms, polygons }` — the carried light's occluded illumination polygon
  (`bound_for_reach` + `visibility_polygon` over `light_walls`, the same primitives as the
  committed field) at each move sample. Computed **only when** the mover carries an enabled
  emission AND the scene is lighting-enabled in `environmentLight` mode — that condition is the
  "cost only on request": one raycast per sample (≤ `MAX_VISION_SAMPLES`), same cost class as the
  existing mover-vision sweep, zero work for lightless moves.
- **Per-recipient admission, not blanket broadcast:** a connection receives `mover_light` only when
  the light can matter to it — the sample's reach disc (position + dim reach) intersects that
  recipient's current vision (committed polygons ∪ their in-flight own-move timelines, the exact
  inputs the existing clip already resolves). A recipient across the map learns nothing; a
  recipient whose corridor the glow spills into gets the timeline. Within an admitted timeline the
  polygons are *not* further clipped — the client intersects them with its own LOS for rendering,
  which it already has. (Residual disclosed: an admitted recipient receives glow geometry slightly
  outside its LOS — light spilling around a corner carries that information physically; logged as
  the invariant-11 trade, fidelity over wire-minimality, with the admission gate bounding the
  radius of disclosure to the light's own reach.)
- **Client:** a lighting sweep beside `visionSweeps`: while a `mover_light` timeline plays, the
  lighting overlay unions the current sample's polygon (cross-faded between samples via the same
  blend machinery fog uses) over the last committed lighting frame; on completion it reverts to the
  derived channel, whose post-commit rebroadcast carries the light's final position. Token
  visibility follows automatically — the mover and any bystander token standing in newly-lit
  cells are darkened-tokens-in-LOS today, so lifting the darkening reveals them mid-stride with no
  token-path changes.
- The existing position-sample clip is unchanged (it gates on LOS, which a moving light does not
  alter); the mover-vision sweep is unchanged (the mover's own fog).

## 9. D9 — Authoring completion

The milestone is unusable without these; all follow the existing tool/panel patterns:

- **Light tool** (scene-tools): click-place a light, drag to reposition, select to edit
  (radii, color, intensity, falloff curve, elevation, enabled), delete. GM-only, mirroring the wall
  tool's intent path. `buildLightDoc` gains its production caller.
- **Wall editing:** select an existing wall → edit `blocksSight` / `blocksMove` / `blocksLight` /
  elevation interval (windows and one-way visibility are the point of `blocksLight`); the wall
  tool's hardcoded all-true defaults stay for creation speed.
- **Actor editor** (`module-actors`): vision-mode assignment list (mode + range per row, replacing
  the darkvision-only input), light emission editor, movement-tag editor.
- **Faction editor** (`module-factions`): movement-tag field.
- **Token editor:** per-token vision / light / movement overrides + elevation input.
- **Settings panel** (`module-game-settings`): vision-mode editor gains `perceives`/`requiresLos`/
  `renderHint` editing and add/remove mode; the illumination-floor dropdown derives from the
  resolved gradation instead of the hardcoded bright/dim/dark list; gradation editor gains
  add/remove band.

## 10. Folded-in fixes (no-deferral rule)

1. **Observer-vision role fork** (`docs/TODO.md`, actionable-now): `player_lit_mask` and
   `gather_vision_sources_in_scene` hand-roll the observer-tier role lookup. Route both through
   `effective_role` (or the capability equivalent) and pin parity with a test exercising both paths
   through the shared symbol — the never-fork rule applied to the subsystem this milestone owns.
2. **`compute_derived` double band resolution** (in-code TODO): `player_lit_mask` already resolves
   bands; the egress payload re-resolves them. Resolve once, pass through.

## 11. Decomposition

| # | Unit | Deliverable |
|---|---|---|
| **M17a** | D1 + D2 + D9-light | Photometric field (additive compose + tint mixing + `FalloffCurve` enum), carried emitters (server + `EffectiveActor`), light tool + light/wall editors, fold-in fix #2. |
| **M17b** | D3 + D9-senses | Vision-mode descriptor v2 + tremorsense seed + `perceived` channel (server + client render), elevation fields + occluder filtering, elevation badge, sense/mode editors, fold-in fix #1. |
| **M17c** | D5 + D9-tags | Movement-type tags end to end: actor/token/faction fields, one resolver, router + executor threading, cost parity, budget interplay, tag editors. |
| **M17d** | D6 | Moving light mid-walk: `mover_light` wire, per-sample computation, per-recipient admission, client lighting sweep. |

Order: **17a → 17b → 17c → 17d** (17d needs 17a's carried lights; 17b's wall-elevation editing
reuses 17a's wall editor; 17c is independent but stays sequential for review cadence). Each unit is
independently testable, gated (full server + client gates), and gets a review pass before the next
begins.

## 12. Security considerations

- The additive field preserves every per-source fail-closed direction; composition only brightens
  for identical content, and brightening is bounded by real authored sources inside the LOS gate —
  the mask remains `LOS ∩ lit`, so no cell outside a source's reach becomes visible.
- Carried lights are computed server-side from the authoritative ECS position; a client cannot
  place a glow anywhere the token isn't. Permission-hidden tokens illuminate (§5, deliberate) but
  are never *named* by any payload; `perceived` is READ-gated (§6.2).
- The elevation model changes only *which walls occlude a source*; a malformed elevation
  (non-finite, bottom > top) fails closed to "blocks everything" (today's behavior) on sight/light
  paths and to 0 (grounded) on tokens.
- `mover_light` admission reuses the clip's existing vision inputs — no new secrecy decision is
  forked; an over-admission bounded by the light's own reach is the sanctioned invariant-11
  residual, stated in §8.
- Movement tags never exempt walls, impassable, arrest, or the visibility mask; the exempt cost is
  computed by the same resolver at preview and execution, so a player cannot preview one price and
  be charged another.
- All new engine fields are `deny_unknown_fields` structs with ts-rs export + Zod mirror (the
  drift guard enforces parity); range/magnitude validation at ingress keeps the DoS bounds
  (`MAX_GATE_WALK_*`, `MAX_VISION_*`, cell-scan caps) binding on the new inputs.

## 13. Testing

- **Server unit:** additive composition (two-light overlap crosses a band threshold; tint mix;
  saturation clamp; env + placed light compose); carried-light union (moves with the token,
  suppresses on `enabled:false`, hidden-token emission illuminates); falloff enum round-trips.
- **Senses:** tremorsense perceives a grounded token through walls in darkness, not a flying one,
  not beyond range, never a permission-hidden one; `perceived` absent for GM/`mode:"all"`;
  elevation band test (see-over, see-under, blocked, malformed fail-closed); observer-fork parity
  test through the shared role symbol.
- **Movement:** exempt mover pays uniform cost on grid and continuous engines (parity with
  executor), impassable/arrest/walls unaffected; faction-union and token-override precedence;
  non-exempt unchanged (existing suites stay green).
- **Mid-walk:** timeline present iff carried-light + environmentLight; admission granted/denied by
  reach-disc ∩ recipient vision; client sweep advances/unions/reverts (unit tests on the blend
  seam; the sweep uses the fog-blend primitives).
- **Client:** `EffectiveActor.light`/`movement` resolution incl. overrides; `toLighting`-adjacent
  parsing of the extended payload; editors write the raw-stored-value OCC pre-image convention.
- **E2e (Playwright):** place a light, see a scene light up for a player; carried torch reveals a
  corridor mid-move; tremorsense renders a token through fog.
- **Gates:** `cargo test` + clippy + fmt, `pnpm -r test`, `typecheck`, `lint`, all docs/comment
  gates, `lint:file-size`, `lint:inline-tests`; ts-rs regeneration committed with each struct
  change.

## 14. Out of scope / excluded

- **Web-Worker optimistic vision** — excluded by PLAN.md; vision stays server-authoritative.
- **Aura / sound / VFX emitters, the generalized emitter component model, emitter editors** — M18;
  M17 delivers the light-emission payload they build on, not the component framework.
- **Impassable/arrest exemptions** (e.g. an ethereal tag passing through walls) — no plan item
  grants them; the tag vocabulary is open for a future reserved tag.
- **Multi-level maps / portals** — Phase 3; elevation here is single-level 2.5D.
- **True gradient lighting render** (the accepted blur-not-gradients and desaturate-approximation
  findings) — previously accepted client-cosmetic approximations, unchanged by this milestone
  unless the sweep work forces a touchpoint.
- **Turn-owner-keyed vision features** — none exist in this design; the M14 dependency line is
  not exercised.
- **Mechanical effects of light level** (dim-light disadvantage etc.) — system-owned rules, as
  before.
