# M10 — Tokens: Cross-Cutting Design Spec

> Status: **DRAFT for review.** Cross-cutting architecture pass over all of M10 (the
> approach used for M8/M9), locking the shared decisions before decomposition. Output
> of the M10 brainstorm (2026-06-24). Stops at the brainstorming review gate; the
> decomposition (§12) feeds per-checkpoint plans via `writing-plans`.

M10 turns the static-image tokens of M8 into the full token feature set: actor-backed
tokens (linked/instanced), world-configurable factions and conditions, per-recipient
name privacy, token shapes/footprints, server-authoritative **pathfinding** (grid +
gridless) with weighted/impassable regions, and the forward-looking **visual stack**
(multi-face, animated, procedurally-generated, fx, emotes) seeded in M8d.

## 1. Goal

Realize the token architecture seeded in M8 ([[token-architecture-forward-looking]],
PLAN M10) on top of M8d's sprite/tween/ticker foundation and M9's wall/vision
geometry — delivering actor-linked tokens, shapes, instanced/unique modes, A*
pathfinding with waypoints, status conditions, and factions, plus name privacy as a
day-one secrecy concern.

## 2. Constraints inherited (cited inline)

- **#1/#3** Server-authoritative; token/region/actor edits are ordinary document
  intents with optimistic-apply + rollback (M5/M6). Ephemerals (emotes, path
  previews, pings) are client-local or transient broadcasts — never documents/ECS.
- **#5** Tokens render from the Zod `DocumentStore` via the M8c reconciler; no client
  ECS.
- **#6** Server stays structural-only — token/actor/region semantics live in the
  opaque `system` body; the server honors **declared** JSON-pointer visibility
  (property overrides) without interpreting meaning. M9's server-authoritative
  geometry (movement-blocking, vision) is the standing exception; M10 adds
  **server-side pathfinding** as an engine-owned consumer of the same `SceneEcs`
  spatial state (already anticipated in `scene/mod.rs`).
- **#7** Canvas engine-owned; tools drive it through the public interaction API.
- **#10** Pointer-based, touch-sized, drag/pinch unified.

## 3. Naming: `Actor` vs `user`

The game entity is named **`Actor`** (`doc_type:"actor"`) — the conventional VTT term,
brand-new so zero churn. The pre-existing user-side "actor" terminology (the
authenticated principal in `permission.rs`/capabilities — mostly doc-comments) is
renamed to **`user`**, unifying with existing vocabulary (`membership`, `as_user`).
Where a spot must distinguish the request's user, "requesting user."

## 4. Actor model + link/instance (M10a)

An **Actor** is a world-scoped `Document`, `doc_type:"actor"`, engine fields in the
opaque `system` body. At M10 it carries only what backs a token — **no sheet, no
stats schema, no browser** (those are M12). `system` shape (client-owned; server
structural-only):

```jsonc
// actor.system
{
  "name": "Goblin Skirmisher",
  "displayName": "Goblin",        // non-secret fallback shown when `name` is hidden (§7)
  "visual": { "kind": "image", "asset": "<uuid>" },  // default token visual (§11)
  "size": { "w": 1, "h": 1 },     // grid units; fractional + multi-cell allowed (§9)
  "shape": "square",              // "square" | "circle" (§9)
  "faction": "<faction-id>",      // → world faction registry (§6)
  "conditions": [],               // → world condition registry (§8)
  "prototype": true               // default place-mode: true ⇒ instance on drop; false ⇒ link
}
```

**Two token→actor modes**, distinguished only by where actor-data lives:

- **Linked (= "unique")** — `token.system.actor_id` references the shared `Actor` doc;
  the token is a live view. `token.system.overrides:{}` may override a small whitelist
  (**name, visual, size**) per-token; transform stays token-local. Actor edits reflect
  immediately. For PCs / unique NPCs.
- **Instanced (= "unlinked")** — on placement the token holds an independent copy at
  `token.embedded["actor"] = [copy]` with `copy.source = {id, pack, version}` (the M2
  provenance). Edits mutate the copy. For monsters/mooks (N independent goblins).

**`prototype`** sets the per-actor default place-mode (GM-overridable at drop via a
tool toggle/modifier).

**`resolveTokenActor(token, store) → EffectiveActor`** is the single read-through every
other system uses (reconciler visual/size, faction border, condition overlays,
pathfinding footprint, `displayName`):
- linked: base = `store.get(actor_id)`, then apply `overrides` (whitelisted keys);
- instanced: base = `token.embedded.actor[0]`.

**Re-sync:** editing a source actor does **not** mutate existing instances (copies).
Provenance is stored so the **deferred** 3-way pull/push merge engine
([[document-inheritance-merge-model]]) can later offer "pull from source." M10 writes
copies-with-provenance only — no merge engine.

**Minimal UI:** a new `src/modules/*` package **`module-actors`** (create/list/pick),
consistent with the M8.5 per-element module pattern. No sheet/browser.

## 5. Token document model (extends M8d §4)

Token `system` (client-owned) keeps M8d's flat transform; gains actor linkage + the
visual union (§11):

```jsonc
// token.system
{
  "x": 0, "y": 0, "rotation": 0,        // transform (center origin); token-local always
  "actor_id": "<uuid> | null",          // linked; null ⇒ instanced (see embedded.actor)
  "overrides": { },                     // linked-only whitelist: name | visual | size
  // visual/size/shape/faction/conditions resolved via resolveTokenActor (§4)
}
```

Effective transform is token-local; everything else flows from `EffectiveActor`.

## 6. Factions (M10b)

A **faction** = `{ name, color, stance: "friendly"|"neutral"|"hostile" }`. Stored in a
**world-scoped config document** (§10), runtime-editable by the GM (add/rename/recolor)
with optimistic UX + broadcast like any document. Seeded with three defaults
(Friendly/Neutral/Hostile, conventional colors) by a replaceable first-party seed
**module**; the GM's runtime edits are authoritative and never overridden by the module.
Each actor/token references a `faction` id. M10 consumes **name + color** (token border
color + group-select); **`stance`** is reserved for later combat/targeting/vision
("reveal hostiles", faction vision-sharing) — present now to avoid a migration.

## 7. Name privacy (M10b) — day-one secrecy

The GM can hide an actor's real name from players; it must never reach an unauthorized
client (retrofitting secrecy leaks). Rides M5 per-recipient property redaction:

- **Mechanism:** "hide name" sets an **`OwnerOrGm`** visibility override on the actor's
  `/system/name` pointer (a **new visibility tier** alongside `GmOnly`; the `Access`
  already knows the doc owner). `filter_properties` / `redact_change` then strip the
  real name on **every** egress path — whole-doc, deltas, and **embedded** (so an
  instanced token's embedded actor copy is covered too; already tested for `GmOnly`).
  The server stays structural-only — it honors a declared pointer, not name semantics.
- **Replacement vs trim:** the non-secret `displayName` field (§4) is always delivered.
  The single display accessor on `EffectiveActor` returns
  `name ?? displayName ?? worldDefault("Unknown Creature")`. GM sets `displayName` to
  replace ("Huge Dragon") or leaves it empty → world-default (trim-to-generic).
- **Authorization:** GM + the actor's **owner** see the real name; everyone else gets
  `displayName`. (Owner tier = why `OwnerOrGm`, not `GmOnly` — a player still sees their
  own hidden PC's name.)
- **Single chokepoint:** because the real name never reaches unauthorized clients,
  every surface (nameplate, hover, future chat speaker, target lists) reads the
  already-redacted actor through the one `displayName` accessor; server-composed
  references (M11 chat) use the per-recipient-filtered doc.
- **Fail-closed** ([[fog-is-the-secrecy-gate-fail-closed]]): the secret is the *absence*
  of data; a garbled/missing state yields no name, never a leak.

## 8. Conditions (M10c) — markers only

No rules/effects engine exists (M11+ dice; no active-effects), so a condition is a
**visual status marker**, not a mechanical effect:

- **Engine mechanism:** a world condition **registry** (config document §10) of
  `{ id, name, icon }`; actor-data `conditions: ConditionId[]`; render as small icon
  **badges** overlaid on the token Container (the [[token-architecture-forward-looking]]
  overlay layer). GM toggles any; a token's owner toggles on their own (capability
  model).
- **Default content as a module:** a self-contained, replaceable first-party
  **`module-conditions`** seeds a generic set (dead, unconscious, prone, stunned,
  poisoned, blinded, invisible, hasted, slowed) into the world registry; a game-system
  module can replace it wholesale. The engine owns the *mechanism*; the module owns the
  *content* (same split as factions).
- **Effects deferred:** mechanical effects land with combat + an effects engine.

## 9. Shapes + footprint (M10d)

- **Shape** ∈ `{ square, circle }` (polygon/custom later), in actor-data + per-token
  override; drives selection/hit-test, the faction-colored border, and the pathfinding
  footprint.
- **Size** in grid units, **fractional → multi-cell** (0.5×0.5, 1×1, 4×4). Continuous
  representation: a token has a footprint **radius** (square via bounding extent /
  circle via radius). This feeds pathfinding clearance/inflation (§10) uniformly across
  sizes.

## 10. Pathfinding (M10e–g)

Server-authoritative, **engine-owned** (queries `SceneEcs` walls/regions — as
`scene/mod.rs` already anticipates), request/response like search. Walls are GM-secret
(M9 ships players only vision polygons), so client-side pathing can't serve players —
this mirrors M9's server-authoritative geometry.

### 10.1 The `Pathfinder` seam

`find(start, goal, waypoints[], footprintRadius, costField, movementModel) → { path[], cost }`

- **`costField`** = hard-obstacle set (walls + impassable regions) **+** an optional
  soft-weight field (weighted regions). Always available on **any** scene — weighting is
  a **region overlay, not a scene mode**.
- **`movementModel`** = `grid-stepped` | `continuous`, a per-scene property (correlates
  with grid kind + a snap setting). This is the real strategy axis (not uniform-vs-
  weighted).

### 10.2 Two engines

| movement model | engine | distance | weighting | footprint |
|---|---|---|---|---|
| **grid-stepped** (snap; D&D/PF) | **grid A\*** (king-moves) | configurable diagonal rule | **per-cell cost (exact)** | clearance grid |
| **continuous** (gridless/wargame) | **`vleue/polyanya`** crate | Euclidean | cost-layers (Split-Mesh; cosmetic boundary approx) | obstacle inflation |

- **Grid A\* (hand-rolled, M10e):** king-moves; cells passable per M9's `blocks_move`
  segment test; **configurable diagonal-cost rule** (a world config-doc setting):
  `chebyshev` (5e, 1-1) | `alternating` (PF1e/3.5, 5-10-5, parity-tracked) | `euclidean`
  (√2) | `manhattan`; **per-cell weights** (exact difficult terrain); clearance footprint
  (anchored footprint cells must be clear). Grid-metric movement (esp. 5-10-5) is defined
  on cell-step sequences, so grid games need cell-stepped A* — **not** any-angle.
  (Theta\*/Anya are therefore dropped: grid games want king-moves, continuous games use
  the crate.) The M8d-3b ruler gains the `alternating` rule.
- **Polyanya (`vleue/polyanya`, M10f):** MIT/Apache, Bevy-free (deps: glam, hashbrown,
  smallvec, spade, geo, bvh2d), ~6.5K Rust SLoC, actively maintained (v0.16.1). Builds
  the navmesh **from edges+obstacles** (CDT via `spade`) and runs the any-angle search;
  **built-in cost layers** (weighted regions), **conditional layers** (dynamic hazard
  toggling), **overlapping layers** (future multi-level maps). We write the adapter
  (scene walls → `Triangulation`; query → path; footprint via inflation), not the
  algorithm. **Caveats (validate in the plan):** meshes are static after construction →
  rebuild on wall/terrain change (same trigger as M9 vision re-derive); adds `geo`+`spade`
  → run the **cargo-bloat** budget check (M9b had deliberately avoided `geo`).
- **Dispatch** by `movementModel`; both behind the seam. Authoritative **movement-
  blocking stays center-based (M9)** in M10 — pathfinding takes `footprintRadius` from the
  request (client resolves `EffectiveActor`), so the server needn't resolve actor-data.
  Footprint-aware *blocking* is a later enhancement.

### 10.3 Weighted/impassable regions (M10g)

- **`region` doc_type** (parent_id = scene; new `SceneEcs` component): `system` =
  `{ shape, cost: number | "impassable" }`. A GM **paint/place tool** authors them.
- Wired into both engines: **per-cell cost** (grid A\*) and **cost-layers** (Polyanya).
  **Hazards** = an impassable region or a toggled conditional layer — satisfying "avoid
  certain paths entirely when they become hazardous" without weighting math.
- Weighting is exact on the grid (per-cell) and approximate-but-cosmetic on the navmesh
  (Split-Mesh boundary refraction — the Weighted Region Problem; visually fine for a VTT).

### 10.4 Client

Waypoint placement (the move/measure tool gains waypoints), a path-preview overlay
(ephemeral, via the §7-style overlay API), and a movement-budget readout. The path is a
correlated request; the move itself remains an optimistic document intent gated by the
M9 server block.

### 10.5 Engine-choice rationale (Polyanya vs A\*+funnel) — recorded

The common anti-Polyanya argument (e.g. Recast/Detour A\* + funnel string-pulling) is
driven by AAA action-game requirements **Shadowcat does not have**, so it does not
override the choice here:

- **Weighting refraction sub-optimality** — moot for the grid majority (grid A\* with
  **exact** per-cell weights; Polyanya isn't in that path); only touches gridless+weighted
  (rare), where "invisible in 99% of games" holds — doubly so for a *human-previewed*
  measurement path, not an autonomous trajectory.
- **Dynamic obstacles** — our walls change on *infrequent GM edits*, re-meshed at the M9
  vision-recompute cadence; not per-frame destructibles.
- **Crowd steering / local avoidance (RVO/DetourCrowd)** — N/A; tokens are human-placed and
  may overlap (VTTs stack tokens).
- **Hundreds-of-units-per-frame performance** — N/A; one path per human drag/waypoint,
  server-side, occasionally.

A\*+funnel is **not** better for this use case: same navmesh requirement (CDT-from-walls),
same rebuild-on-edit story, **also** approximate under weighting (the unsolved Weighted
Region Problem — both engines miss optimality in opposite ways), an irrelevant performance
win, an unneeded crowd-steering feature — and **higher build cost** (hand-roll funnel +
navmesh-A\*, or a C++ Recast binding that complicates the pure-Rust cross-platform build +
the cargo-bloat budget) versus the ready, maintained `vleue/polyanya` crate. The one real
(mild) caveat — navmesh rebuild on wall edits — is engine-independent and validated in the
M10f plan.

## 11. Visual stack (M10h–j)

`TokenNodeSpec.visual` generalizes from flat `{kind:"image", asset}` to a discriminated
union the reconciler renders per kind; a **per-token** filter/overlay attach point is
added (today's `addLayerFilter` is per-layer).

- **`image`** — unchanged (ships today).
- **`faces`** (M10h) — `{kind:"faces", faces:{name→asset}, current}`; reconciler swaps
  texture by `current`, set by a state field/condition or manual GM/owner action.
- **`animated`** (M10h) — `{kind:"animated", source: spritesheet|frames, fps, loop}` →
  PixiJS `AnimatedSprite` driven by the existing render ticker (`TokenView.tick`).
- **`generated`** (M10i) — a richer **parametric** generator (shape + color + border +
  icon/initial + simple layering) → render-to-texture; gives artless actors a sensible
  default token. Rich third-party generators slot the same kind later (modular). *(Own
  checkpoint per the milestone-decomposition latitude.)*
- **fx** (M10j) — per-token **built-in** Pixi filters: tint/alpha/desaturate
  (condition-driven, e.g. dead → grayscale) + selection/faction/target highlight. The
  **custom shader-filter seam stays Phase-3 VFX** (no other consumer yet).
- **emotes** (M10j) — transient overlay above the token (bubble/reaction), ephemeral via
  a **ping-style transient broadcast** (`ping-view` is the model; a new `emote` aux frame
  mirrors `Ping`/`AssetChanged`); renders an overlay child + fades.

## 12. Decomposition (approved 2026-06-24)

One M10 cross-cutting spec (this doc) → per-checkpoint plans; **`/clear` between each**;
each independently shippable + buddy-checked (M8/M9 cadence). Dependency order is
linear (a → b/c → d → e → f → g → h → i → j).

**Phase 1 — Actor foundation**
- **M10a — Actor model:** `user` rename; `Actor` doc_type; link/instance + overrides +
  provenance; `resolveTokenActor`; `module-actors` create/list/pick; place-tool wiring.

**Phase 2 — Token identity & decoration**
- **M10b — Factions + name privacy:** faction config-doc registry + seed module + border
  + group-select; `OwnerOrGm` tier + `displayName` + redaction + accessor.
- **M10c — Conditions:** condition config-doc registry + replaceable `module-conditions`
  default set; `conditions[]`; overlay badges; GM+owner toggle (markers-only).

**Phase 3 — Geometry & movement**
- **M10d — Shapes + footprint:** shape `{square,circle}` + size (fractional→multi-cell) +
  override; footprint radius.
- **M10e — Pathfinding (grid):** `Pathfinder` seam + server request/response + grid A\*
  (configurable diagonal rule config-doc, clearance) + waypoints + preview + budget;
  ruler gains `alternating`.
- **M10f — Pathfinding (continuous):** adopt `vleue/polyanya`; walls→Triangulation
  adapter; gridless movement; movement-model dispatch.
- **M10g — Weighted/impassable regions:** `region` doc_type + ECS component + GM paint
  tool; weighting into both engines; hazard toggling.

**Phase 4 — Visual stack**
- **M10h — faces + animated:** `visual` union; `faces` + `animated`.
- **M10i — generated:** parametric token generator → render-to-texture.
- **M10j — fx + emotes:** per-token built-in filters + emotes (transient overlay).

## 13. Decisions — CONFIRMED (user, 2026-06-24)

1. Cross-cutting pass first → 10-checkpoint decomposition (§12), approved as-is.
2. Actor scope: thin `Actor` doc + minimal picker; sheets/stats/browser = M12.
3. Naming: game = `Actor`; user-side → `user`.
4. Link model + minimal per-token overrides (name/visual/size) on linked tokens.
5. Factions: config-doc `{name,color,stance}`, 3 seeded defaults, runtime-editable,
   seed module; stance reserved.
6. Conditions: engine registry + replaceable default-content module; markers-only.
7. Name privacy: `OwnerOrGm` tier + `displayName` fallback; fail-closed.
8. Pathfinding: server-side engine-owned; **two engines** (grid A\* + `vleue/polyanya`)
   by movement model; weighting universal (region overlay); footprint-aware
   (clearance/inflation incl. fractional); configurable diagonal rules; region authoring
   shipped (M10g); Theta\*/Anya dropped. **Dependency `vleue/polyanya` adopted**
   (bloat-check pending).
9. Visual stack: `faces` + `animated` + richer `generated` + per-token built-in fx
   (shader seam stays Phase-3) + emotes.
10. World registries stored as world-scoped **config documents**.

## 14. Out of scope / deferred

- Actor sheets / stats schema / actor browser / compendium actors (M12).
- Mechanical condition **effects**; an effects/rules engine (combat milestones).
- Weighted-region **boundary-exact** (Snell) pathfinding; weighted *continuous* beyond
  Split-Mesh cost-layers.
- Footprint-aware **movement-blocking** (M10 blocking stays center-based, M9).
- Custom **per-token shader** fx (Phase-3 VFX); aura/light/sound/VFX **emitters**, trigger
  regions, token-art enrichment (Phase 2).
- The 3-way pull/push **merge engine** for instanced-token re-sync (data model only).
- Grid-locked stepped movement as a separate continuous-scene option; weighted grid-A\*
  variants beyond the configured diagonal rule.
- Multi-level maps / overlapping navmesh layers (Phase 3; the crate admits them).
