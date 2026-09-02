# M18 — Token Enrichment — Brainstorm

> Date: 2026-08-31. Roadmap entry: `docs/PLAN.md` §M18. Work happens on branch
> `m18-token-enrichment` (worktree `Shadowcat-m18`, based on `main` = M14c-3 merge).
> Other agents are concurrently landing m15b (asset browser), m15 (move hardening),
> m16 (layout/theming), and m17 (vision/lighting/movement); collision surface is
> mapped in §3.

## 1. Scope (from PLAN.md, verbatim intent)

1. **Aura / light / sound / VFX emitters as token components** — the component model
   lands here; sound and VFX emit into the Phase-3 audio/VFX seams.
2. **Trigger regions** — mechanical/trigger effects on the M10g region primitive:
   damage, condition application, scripted triggers on enter/arrest.
3. **Token art tooling.**
4. **Generated token visuals** (deferred from M10i) — a parametric compositor that
   frames existing actor art: decorative border + shape-crop mask + background,
   distinct from the dynamic faction ring; additive `{kind:"generated"}` on the
   M10h `RenderVisual` union.
5. **Per-token built-in fx** (deferred from M10j) — condition-driven
   tint/desaturate/highlight + selection/faction/target highlight via a per-token
   Pixi `.filters` attach point on the M10h token `Container`.
6. **Emote / reaction overlays** (deferred from M10j) — transient overlay above the
   token via a new ping-style `emote` aux frame + fading child.

Depends on: M14 (combat clock — done through M14c-3 on main; condition/damage
primitives exist), M17 (light emitters — **in flight**, see §3).

## 2. What exploration established (state of `main`)

- **Emitter precedent already exists (m17, unmerged):** `LightEmission` payload
  (position-free), `ActorEngine.light` default + `TokenOverrides.light` wholesale
  override, `enabled:false` suppress, GM-only value-aware permission predicate
  (`carried_light_touched`), single read path
  (`SceneEcs::token_light_emission` → `SceneEcs::scene_lights`). m17 does **not**
  touch `TokenVisual`/`RenderVisual`.
- **Trigger-region runway is clear:** no in-flight branch touches `move_exec.rs`,
  `regions.rs`, `pathfinding.rs`. But: `RegionField` erases region identity after
  composition; `move_exec` discards the entered-cell sequence (`MoveOutcome`
  carries only stop/path/cost); the natural fire point is `Room::execute_move`
  after the position commit, inside `publish_guard` — the combat movement-budget
  decrement (room.rs, `WriteOrigin::CombatTransition`) is the exact precedent for
  a server-authored write from move execution. The ECS stays read-only.
- **Damage primitive:** only combat resources. `combat::transition::resource`
  (snapshot-bound, `ResourceOp::Delta|Set`, clamps `[0,max]`, refuses `Mirror`
  bindings). No damage concept exists outside combat — damage is system-owned
  (M14 exclusion). `resource-registry` is empty by default.
- **Conditions:** plain data (`ActorEngine.conditions: Vec<String>`); no server
  apply-helper, but the write paths are known (linked → actor
  `/engine/conditions`; instanced → token `/embedded/actor/0/engine/conditions`;
  host join precedent `combat::eval::formula_host`).
- **Formula engine:** `combat::eval::eval_formula` + `SystemLeafResolver` give
  server-side formula evaluation against a host document — reusable for trigger
  amounts verbatim.
- **Hooks:** no server hook bus; client `HookBus`/`CoreHooks` exists (merged,
  empty). M14c-6 plans delta-derived CoreHooks emission — document writes are the
  deltas modules observe.
- **Secrecy:** secret (`gm_only`) regions are sprung by the authoritative field at
  execution; any public side-channel naming the region is a leak. Combat precedent:
  `Audience::GmOnly` notices. Aux frames (`broadcast_aux`) are room-wide — they
  cannot carry secret-region information.
- **Render:** `TokenNode` = `container` (non-rotating; badges) → `visualContainer`
  (rotating; `visual` + faction `border`). No per-token filter API yet; pixi
  8.19 ships `ColorMatrixFilter` (tint/grayscale/saturation). Selection ring is an
  ephemeral overlay `Graphics`; faction ring is the 3px `border` Graphics; target
  highlight does not exist (no targeting feature anywhere). `ScenePing` is the
  end-to-end aux-frame template (9 touch points, enumerated).
- **Authoring UI:** `VisualKindEditor` is create-form-only on main; m15b rewrote it
  onto `ctx.pickAsset` (which does not exist on main). Editor work must target the
  m15b shape → sequence it last, after merging main (with m15b) into m18.

## 3. Collision surface with in-flight branches

| M18 work | Overlap | Severity |
|---|---|---|
| Emitter components | m17: `token.rs`, `permission.rs`, `command.rs`, `ActorsPanel.svelte` | High but additive/mechanical; must **generalize m17's pattern, never fork a second emitter model** |
| Trigger regions | none | Low — clean runway |
| Generated visuals | `VisualKindEditor.svelte` (m15b rewrite) | Low–medium |
| Per-token fx | `engine.ts` (m16 `setThemeColors`, m17 `LightView` wiring) | Medium, additive |
| Emotes | none ("emote" on main is only a chat `MessageKind`) | Low |
| Art tooling | m15b `appContext.ts`/`VisualKindEditor.svelte` | Medium — target `pickAsset` |
| ui-kit locales/index | all branches | Mechanical merges |

## 4. Design decisions (best long-term shape; options considered)

### D1 — Emitter component model: typed sibling fields, m17 pattern generalized
**Decision:** the component model IS the m17 precedent, generalized by repetition
with typed payloads: each emitter kind is an `Option<T>` field on `ActorEngine`
(default) + `TokenOverrides` (wholesale override), with `enabled:false` suppress,
a GM-only value-aware permission predicate, and a single resolver mirroring
`resolveTokenActor` precedence. M18 adds:
- `AuraEmission { color, opacity, radius, enabled }` — radius in grid cells;
  rendered client-side (soft filled disc under the token art, above the base
  map). Purely visual; no mechanics (mechanics are trigger regions' job).
- `SoundEmission { ... }` / `VfxEmission { ... }` — **data model + resolution +
  permission seams only.** Playback is Phase 3 (audio/VFX milestones in PLAN.md —
  the one sanctioned blocker class; PLAN.md's own M18 text scopes these as
  "emit into the Phase-3 seams; the component model lands here"). Payloads are
  designed so Phase-3 playback needs no schema change.
- Light stays m17's; on merge the kinds coexist as siblings. No components map:
  with 3–4 kinds, typed fields give `deny_unknown_fields` ingress validation and
  match the typed-`engine`-band philosophy; a string-keyed map would re-open the
  opaque-`system` validation hole for engine-owned data.

### D2 — Trigger regions: effects on `RegionEngine`, fired from `Room::execute_move`
- `RegionEngine` gains `triggers: Vec<RegionTrigger>` (serde default, additive).
  `RegionTrigger { on: "enter" | "arrest", effect: TriggerEffect }`;
  `TriggerEffect = ConditionAdd { condition } | ConditionRemove { condition } |
  ResourceDelta { resource, amount: Formula } | ChatNotice { text, audience }`.
  Triggers are orthogonal to `behavior` (a terrain region can also burn you).
- **"Scripted triggers" = formula-scripted parameters** (`amount` is a `Formula`
  evaluated server-side via `combat::eval::eval_formula` against the entering
  token's actor host) **+ module observability through document deltas** (the
  CoreHooks delta-derived emission M14c-6 establishes). A bare "fire a named
  hook" effect kind is rejected: no server hook bus exists, aux frames are
  room-wide (secrecy leak), and an effect that writes nothing is unobservable —
  every effect here writes a document, which IS the scriptable signal.
- **Identity:** the composed `RegionField` stays the movement authority; a new
  per-region side-table (`region_id → cell set`, same rasterization) answers
  "which trigger regions did this entered cell belong to". One rasterizer feeds
  both — never two geometry derivations.
- **Executor purity preserved:** `move_exec` gains an entered-cell report on
  `MoveOutcome` (deduped cell transitions already exist in the step loop); it
  writes nothing. `Room::execute_move` maps cells → trigger regions and fires
  after the position commit, inside `publish_guard`, via `commit_ops_locked`
  under `WriteOrigin::CombatTransition` — never mixed into the client-origin
  position batch (the room.rs batching discipline).
- **Damage = `ResourceDelta`, combat-scoped.** Outside an active combat (or for a
  token with no combatant) a resource effect no-ops with a `Audience::GmOnly`
  notice — there is no engine-owned damage pool outside combat by design (damage
  automation is system-owned per M14's exclusion). Condition effects work always.
- **Secrecy:** effects on `gm_only` regions fire normally (the server springs
  secrets — established semantics) but every notice they produce is forced
  `Audience::GmOnly`; condition/resource writes are document updates, already
  egress-filtered per recipient. No aux frame names a region.
- **Entry is entry, regardless of transport:** triggers fire on executed moves
  (the `Room::execute_move` site) AND on token placement/teleport (a token Create
  or an `/engine/x`/`/engine/y` position write that lands the footprint inside an
  armed trigger region) — a GM placing a token into a fire region and seeing
  nothing happen is the silent-gap semantics this design rejects. Both fire sites
  share one effect-application helper; the placement site hooks the Room commit
  tail for token position writes, reusing the same cell→region lookup (the
  token's new footprint cells ∩ trigger-region cells). A region doc being
  created/edited under already-standing tokens does NOT re-fire (the token did
  not enter; the region moved).
- Validation: `RegionEngine` currently has no semantic validate (read-side
  fail-closed). Trigger payloads get real validation on ingress
  (condition/resource id shape, formula parse-check, audience vocabulary) since
  they are engine-executed, unlike the passive movement fields.

### D3 — Generated token visuals: compositor on `RenderVisual`
`{kind:"generated", art: Box<RenderVisual>, crop: "circle"|"square", border?:
GeneratedBorder { color, width }, background?: { color } }`. `art` may be
`image`/`animated` only — never `faces`, never nested `generated` (read-side
fail-closed, same guard shape as `resolveTokenVisual`'s existing checks).
Because it sits on `RenderVisual`, it works top-level AND per-face for free.
Rendering: pixi-backend composes background fill → masked art sprite →
decorative border `Graphics` inside `visualContainer` (rotates with art); the
runtime faction ring stays the outer `border` — visually and structurally
distinct. `TokenNodeSpec.visual` gains a resolved `generated` arm (urls
pre-resolved; the backend never resolves asset ids — standing rule).
`resolveTokenVisual` accepts `generated` (validating `art`), old clients fail
closed to nothing — the M10h forward-compat seam working as designed.

### D4 — Per-token built-in fx: one spec-driven filter channel, data-driven sources
- New per-token filter attach point: `TokenNodeSpec.fx?: TokenFx[]`,
  `TokenFx = {kind:"tint", color, strength} | {kind:"desaturate"} |
  {kind:"highlight", color, strength}`, applied to `visualContainer.filters`
  (art fx rotate with the art; badges stay clean). `DisplayBackend` gains the
  seam; `MockBackend` records it; `ColorMatrixFilter` is the only primitive.
- **Condition-driven fx are data-driven:** `Condition` gains optional `fx` —
  `{ tint?: css-color, desaturate?: boolean, highlight?: css-color }` — authored
  per-condition in the registry (ConditionsPanel editor). token-view folds the
  token's resolved conditions' fx into `TokenNodeSpec.fx` (deterministic order:
  registry id order; multiple tints compose by Pixi's filter chain).
- **Highlights:** selection highlight moves onto the token node through the fx
  channel (replaces the ephemeral overlay ring as the selection signifier —
  one mechanism, not two). The faction ring is identity, not a highlight — it
  stays the `border` Graphics. **"Target" is a named highlight source the
  channel accepts with no producer yet** — no targeting feature exists in any
  milestone; building one is far outside M18's text. The channel is the
  deliverable; targeting plugs in without render-layer changes when it lands.

### D5 — Emote overlays: ping-pattern aux frame, ownership-gated
`ClientMsg::Emote { scene, token, emote }` → authz (effective token owner or GM,
mirroring the move gate's ownership rule) + rate limit (ping's 30/min pattern) →
`ServerMsg::Emote { scene, token, user, emote }` room-wide aux → client filters
on `viewedSceneId` (the cross-scene leak guard) → rising/fading emoji `Text`
above the token (~2s, overlays-layer sibling of pings). `emote` is a short glyph
string, server-validated (≤ 4 graphemes-ish bound, non-empty). A bounded
GM/system-defined set is rejected — free emoji is the Foundry-parity behavior
and needs no registry. UI: emote palette on the scene-tools rail acting on the
selected token(s).

### D6 — Token art tooling: editor for existing visuals + generated authoring
The gap: `VisualKindEditor` is create-form-only. M18 makes visual authoring
available post-create — per-actor (edit the actor's visual) and per-token
(override via `TokenOverrides.visual`), with the `generated` kind added to the
kind selector — all through `ctx.pickAsset` (m15b shape). **Sequenced last**;
before starting it, merge `main` (with m15b) into the m18 branch. If m15b is
still unmerged at that point, that is a cross-agent dependency to surface, not
to code around against the vanishing main-era inline-grid API.

## 5. Sub-project decomposition (build order)

| # | Sub-project | Contents | Depends on |
|---|---|---|---|
| M18a | Emitter component model | `AuraEmission`/`SoundEmission`/`VfxEmission` server types + validation + permission predicates + ECS resolvers; aura client render; actor/token authoring UI | none (additive beside m17) |
| M18b | Trigger regions | `RegionEngine.triggers` + validation; region-identity side-table; `MoveOutcome` entered-cell report; firing in `Room::execute_move`; condition/resource/notice effects; GM-only secrecy discipline; tests | M14 (on main) |
| M18c | Generated token visuals | `RenderVisual::Generated` (Rust + ts-rs + client); pixi compositor; `resolveTokenVisual` acceptance | none |
| M18d | Per-token fx | `TokenNodeSpec.fx` + backend seam; `Condition.fx` registry fields + ConditionsPanel editor; token-view folding; selection-highlight move | none |
| M18e | Emote overlays | aux frame pair end-to-end (protocol, ts-rs, Zod, conn handler, room broadcast, session guard, render spawn/fade); tools-rail palette | none |
| M18f | Token art tooling | post-create visual editing (actor + token override), generated-kind editor, `pickAsset` integration | M18c, m15b on main |

Order of execution: M18b and M18c first (clean runways, no in-flight overlap),
then M18a (additive beside m17), then M18d/M18e, M18f last.

## 6. Explicit exclusions (ratified or proposed)

- Sound/VFX **playback** — Phase 3 (PLAN.md's own scoping).
- Region-doc edits re-firing triggers on stationary tokens — the region moved,
  the token did not enter.
- Targeting feature — not built; the fx channel names `"target"` as a source.
- Photometric light behavior — m17's; M18 touches nothing m17 owns beyond
  additive sibling fields.
- Custom shader-filter seam — stays Phase 3 VFX (PLAN.md M18 text).

## 7. Open questions surfaced (none blocking)

None that change the build order. The judgment calls a reviewer should weigh:
(a) D2's dual fire sites (move execution + placement/teleport, region edits
excluded); (b) D4's selection-ring replacement and producer-less `"target"`
source. Both are stated above with rationale; both follow the codebase's
one-mechanism discipline.
