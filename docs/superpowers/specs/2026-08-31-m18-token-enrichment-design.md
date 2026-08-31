# M18 — Token Enrichment — Design Spec

> Parent brainstorm: `2026-08-31-m18-token-enrichment-brainstorm.md` (decisions
> D1–D6, collision map, exclusions). This spec fixes the concrete shapes each
> sub-project implements. Sub-project order: M18c, M18b, M18a, M18d, M18e, M18f.

## Common rules

- Server types are Rust in `src/server/src/data/engine/` with
  `#[serde(deny_unknown_fields)]` where serde allows (internally-tagged enums
  cannot carry it — the `TokenVisual` precedent), ts-rs `TS` derives, docs on
  every field (`#![deny(missing_docs)]` is live in these modules). Regenerate
  `src/types/generated/**` from the Rust; never hand-edit.
- Engine-owned data lives on the typed `engine` band, validated by
  `validate_engine`/`normalize_engine` — never on the opaque `system` band.
- Comments cite symbols, never files/lines; no milestone ids, dates, or history
  narration in code comments (RULE 15/16, `docs/design/doc-sweep-truthfulness-rules.md`).
- Fail closed everywhere: malformed payloads resolve to "no effect" / "no
  render", never to a default that grants something.

## M18c — Generated token visuals

### Data (`data::engine::token`)

```rust
/// additive member of the RenderVisual union
Generated {
    art: Box<RenderVisual>,              // Image | Animated only (read-side guard)
    crop: GeneratedCrop,                 // "circle" | "square"
    border: Option<GeneratedBorder>,     // decorative frame
    background: Option<GeneratedBackground>,
}
GeneratedCrop      = enum { Circle, Square }      // serde lowercase
GeneratedBorder    { color: String, width: f64 }  // css #rrggbb, width in token-fraction px
GeneratedBackground { color: String }             // css #rrggbb
```

- Read-side guard: `art` resolving to `Generated` (nested) or, at the
  `TokenVisual` boundary, anything not `Image`/`Animated` fails closed to
  `null` in `resolveTokenVisual` — same guard shape as the existing
  malformed-`faces`/`AnimatedSource` checks.
- ts-rs regen → `RenderVisual.ts`, new `GeneratedCrop`/`GeneratedBorder`/
  `GeneratedBackground` files, `src/types/index.ts` export lines.

### Client resolution + render

- `resolveTokenVisual` (`src/client/core/src/actor.ts`) accepts `generated`
  after validating `art` (recursively through the same validity checks; cycle
  depth is structurally capped because `art` cannot be `faces` and nested
  `generated` is refused).
- `TokenNodeSpec.visual` gains
  `{kind:"generated", art: ResolvedTokenVisual /* image|animated arm */, crop,
  border?, background?}` — urls pre-resolved by `TokenView.toSpec`'s
  `resolveSource` path (the backend never resolves asset ids).
- `PixiBackend` composes, inside `visualContainer`: `background` fill
  (`Graphics`, token-extent sized, cropped shape) → art sprite masked by the
  crop shape (`Graphics` mask; circle = inscribed ellipse of the token extent,
  square = the extent rect) → decorative `border` ring (distinct from the outer
  faction `border` Graphics, which is untouched).
- `visualSourceKey` covers the new arm (json of crop/border/background + art
  key) so tween-only pushes still short-circuit; the async
  object-identity guard invariant applies to the composed children exactly as
  for `replaceVisualChild` today.
- `MockBackend` records the new arm structurally (no GL needed).

### Tests

- Rust: serde round-trip, ts-rs shape.
- `actor.test`: resolveTokenVisual accepts valid generated, fails closed on
  nested generated / missing art / bad animated art.
- `token-view`/`pixi-backend` (via MockBackend): spec carries the resolved arm.
- e2e (`stage.spec.ts` convention): author a generated-visual actor, place,
  assert a `data-*` signal reflecting the resolved visual kind.

## M18b — Trigger regions

### Data (`data::engine::geometry` — RegionEngine)

```rust
RegionEngine { shape, behavior, cost, enabled,
               triggers: Vec<RegionTrigger> }        // #[serde(default)]

RegionTrigger { on: TriggerEvent, effect: TriggerEffect }
TriggerEvent  = enum { Enter, Arrest }              // serde snake_case
TriggerEffect = enum (serde tag "type", snake_case) {
    ConditionAdd    { condition: String },
    ConditionRemove { condition: String },
    ResourceDelta   { resource: String, amount: Formula },  // crate Formula enum
    ChatNotice      { text: String, audience: NoticeAudience },
}
NoticeAudience = enum { Public, GmOnly, Owner }     // Owner = token's effective owner + GMs
```

- Ingress validation (new — these are engine-EXECUTED payloads, unlike the
  passive movement fields): condition/resource ids non-empty + bounded length;
  `amount` parse-checked via `formula::parse` at validate time; `text` bounded;
  unknown `type`/`on` rejected by serde. Wired into `normalize_engine`'s
  `"region"` arm (real validation, not the current plain round-trip — the
  movement fields keep their read-side fail-closed semantics unchanged).

### Server machinery

- **Region identity:** new side-table beside the composed `RegionField`:
  `region_cells(scene) -> Vec<(region_id, region_doc_hash, CellSet)>` built from
  the SAME rasterization (`regions::rasterize`) the composed field uses — one
  geometry derivation feeding both consumers. Rebuilt on region-doc mutation
  like the existing region side-table. Answers: "which enabled trigger-bearing
  regions cover this footprint cell set?"
- **Executor report:** `MoveOutcome` gains `entered_cells: Vec<Cell>` (the
  deduped cell-transition sequence the step loop already computes, capped —
  reuse an existing cap constant shape like `MAX_GATE_WALK_COORD`-bounded path
  length). `move_exec` stays pure: it reports, never writes.
- **Fire site 1 — move execution:** `Room::execute_move`, after the position
  commit, inside `publish_guard`, same slot shape as the combat movement-budget
  decrement. Cells → armed regions (identity side-table, authoritative — the
  server springs secrets). For each region: `Enter` effects fire on any entered
  cell; `Arrest` effects fire only when the walk was arrested inside that
  region (`stopped_early` + arrest cell membership).
- **Fire site 2 — placement/teleport:** the Room commit tail for token Creates
  and `/engine/x`/`/engine/y` position Updates: compute the token's new
  footprint cells, intersect armed trigger regions, fire `Enter` effects.
  Region-doc edits never re-fire. One shared effect-application helper serves
  both sites.
- **Effect application** (single helper, server-authored ops via
  `commit_ops_locked`, `WriteOrigin::CombatTransition`, never batched with the
  client-origin position write):
  - `ConditionAdd/Remove`: resolve the host (token-embedded actor copy first,
    else linked actor — the `combat::eval::formula_host` join), build an OCC
    `Operation::Update` on the correct conditions path; idempotent (adding a
    present condition / removing an absent one is a no-op).
  - `ResourceDelta`: combat-scoped — resolve the token's combatant in the
    world's ACTIVE combat; evaluate `amount` via `combat::eval::eval_formula`
    against the actor host; apply `ResourceOp::Delta`. No active combat / no
    combatant / Mirror binding → no-op + `Audience::GmOnly` notice.
  - `ChatNotice`: post via the chat ingest path; audience forced `GmOnly` when
    the region is not visible to all recipients (secrecy: no public
    side-channel may name or imply a secret region). `Owner` audience =
    effective owner + GMs.
- **Dedup:** one region fires each matching effect at most once per move /
  placement (multiple entered cells of the same region collapse).

### Client

- Region tool: trigger authoring UI on region creation (effect list editor:
  type, on, fields) — minimal but real; region edit UI remains
  delete+recreate per the standing convention.
- `ChatNotice` output appears through the normal chat pipeline (no new render).

### Tests

- regions.rs: identity side-table build, same-rasterization parity with the
  composed field (anti-drift test), disabled regions absent.
- move_exec: entered_cells report exactness (dedup, arrest truncation).
- Room: Enter fires on move; Arrest fires only on arrest; placement fires;
  region edit does not; condition add/remove idempotence; ResourceDelta in/out
  of combat; secret region → GM-only notice, nothing public; formula amount
  evaluated against the entering token's actor.
- Ingress validation rejects malformed triggers; old docs without `triggers`
  load (serde default).

## M18a — Emitter component model (aura / sound / VFX)

### Data (`data::engine::token`, sibling to m17's `light`)

```rust
ActorEngine:     aura: Option<AuraEmission>, sound: Option<SoundEmission>,
                 vfx: Option<VfxEmission>            // all #[serde(default)]
TokenOverrides:  same three Option fields             // wholesale override
AuraEmission  { color: String, opacity: f64, radius: f64, enabled: bool }
                                                 // radius in grid cells, opacity 0..=1
SoundEmission { asset: String, radius: f64, volume: f64, loop_: bool, enabled: bool }
VfxEmission   { asset: String, anchor: VfxAnchor, loop_: bool, enabled: bool }
VfxAnchor     = enum { Token, Above, Below }     // serde snake_case
```

- Sound/VFX payloads are playback-ready (asset id, spatial/volume fields) but
  NOTHING plays them — Phase 3 owns playback (PLAN.md's own scoping). No client
  render for sound/vfx in M18.
- Validation: real `validate` arms — css color shape, finite bounded numbers,
  opacity/volume clamped read-side like m17's intensity.
- Permissions: GM-only value-aware predicates mirroring m17's
  `carried_light_touched` shape (`/engine/aura` etc. on actor,
  `/engine/overrides/aura` on token, ancestor-write subtree comparison).
- Resolution: `EffectiveActor` projections `aura`/`sound`/`vfx` with the same
  precedence as every overridable field (override replaces base wholesale);
  server ECS mirrors for future consumers.

### Client render (aura only)

- Aura draws UNDER the token art, above the map: a soft filled disc
  (color + opacity, radius × cell size) in the token layer, sibling child of
  `container` drawn before `visualContainer`; hex scenes use the same circle
  (auras are radial by definition). Reacts to grid size through the same
  cell-size source the token extent uses — never a second grid formula.
- Authoring UI: per-actor + per-token emission editors in the actors module,
  shaped like m17's `TokenLightControl`/`LightEmissionEditor` (which M18 does
  not modify — additive siblings; on merge with m17 both sets of fields
  coexist).

## M18d — Per-token built-in fx

### Render seam

```ts
// TokenNodeSpec addition
fx?: TokenFx[]
type TokenFx =
  | { kind: "tint"; color: number; strength: number }       // 0..1
  | { kind: "desaturate" }
  | { kind: "highlight"; color: number; strength: number }  // brighten-toward
```

- Applied to `node.visualContainer.filters` (art fx rotate with the art;
  badges stay clean). `ColorMatrixFilter` only (tint / grayscale / brightness
  compose). `DisplayBackend` gains the per-token seam; `MockBackend` records.
- Filter instances are rebuilt only when the fx KEY changes (same memoization
  discipline as `badgeKey`/`sourceKey`); dispose correctly on token removal.

### Data-driven condition fx

- `Condition` (registry entry) gains `fx?: { tint?: string; desaturate?:
  boolean; highlight?: string }` (css colors) — Rust
  `ConditionRegistryEngine` + ts-rs + ConditionsPanel editor fields.
- `token-view` folds resolved conditions' fx into `TokenNodeSpec.fx`,
  deterministic order (condition array order — same order the face map reads).

### Selection highlight

- The selection signifier moves from the ephemeral overlay ring onto the token
  node: selected tokens get a `highlight` fx entry (accent color
  `0xffd400` preserved) pushed by `token-view` from the selection state the
  view already receives. The overlay-ring draw in the scene-tools controller is
  removed (one highlight mechanism — churn accepted deliberately).
- The fx channel names `highlight` sources structurally; `"target"` is a
  reserved source with no producer (no targeting feature exists — the seam,
  not the feature, is M18's deliverable).

## M18e — Emote overlays

### Wire (template: ScenePing, all 9 touch points)

```rust
ClientMsg::Emote { scene: Uuid, token: Uuid, emote: String }
ServerMsg::Emote { scene: Uuid, token: Uuid, user: Uuid, emote: String }
```

- conn handler: rate check (ping's per-user pattern, separate bucket) → authz
  (effective token owner or GM — `effective_owner`, fail-closed) → emote
  validation (non-empty, ≤ 16 bytes — covers 1–4 emoji graphemes) →
  `broadcast_aux`. Room-wide like ping; NO secrecy stake (an emote reveals only
  that a visible token emoted — a token the recipient cannot see renders no
  overlay because the client drops overlays for unknown token ids).
- Client: Zod member (all fields declared — z.object strips), `WsClient`
  dispatch + handler type, `WorldSession` `onEmote`/`sendEmote` with the
  `viewedSceneId` cross-scene guard, AppContext bridge.

### Render + UI

- Render: `EmoteView` (ping-view pattern, pure age-tracker) — emoji `Text`
  spawned above the token's current position, rises ~1 cell and fades over
  ~2s; token position re-read each frame so a moving token's emote tracks its
  start point (deliberate: the emote marks WHERE it was fired).
- UI: emote palette in the scene-tools rail (grid of common emoji + free
  input), acting on the selected token; disabled without a selection the user
  effectively owns (client-advisory; server enforces).

## M18f — Token art tooling

- `VisualKindEditor` gains the `generated` kind (art pick via
  `ctx.pickAsset({kind:"image"})` or an animated source, crop select, border
  color/width, background color, live completeness validation in `buildVisual`'s
  per-kind pattern).
- Post-create editing: the actors panel gains "edit visual" for an existing
  actor (reuses `VisualKindEditor` initialized FROM the actor's current visual
  — the editor learns an `initial?: TokenVisual` prop) and per-token override
  editing (writes `TokenOverrides.visual`, raw-`old` OCC convention).
- **Prerequisite:** merge `main` into `m18-token-enrichment` once m15b lands
  (the editor targets `ctx.pickAsset`; main has no such API). If m15b is still
  unmerged when M18f starts, surface it — do not build against the vanishing
  main-era inline-grid API.

## Verification (every sub-project)

- `cargo test` (server, from `src/server/`), `cargo clippy`, `cargo fmt`.
- `pnpm -r test`, `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`,
  `pnpm lint:props`, `pnpm lint:comments`, `pnpm lint:file-size`,
  `pnpm lint:inline-tests`, `pnpm docs:check-examples`.
- `pnpm build` before any cargo build of the server (rust-embed compile-time
  check of `dist/`).
- Skill-update gate: `shadowcat-codebase-actors-tokens`,
  `shadowcat-codebase-scene-rendering` (+ `-combat`, `-dice` if their seams
  move) updated in the same commits; symbol-citation gate must pass
  (`node scripts/check-skill-symbol-refs-cli.mjs`).
