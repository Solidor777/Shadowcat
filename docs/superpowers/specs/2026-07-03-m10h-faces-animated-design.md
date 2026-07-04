# M10h — Faces + Animated Tokens: Design Spec

> Status: **APPROVED** (user, 2026-07-03). First checkpoint of M10's Phase 4 visual
> stack (parent spec `docs/superpowers/specs/2026-06-24-m10-tokens-design.md` §11).
> Purely client-side — no server/ts-rs change.

## 1. Goal

Generalize the token `visual` from a flat `{kind:"image", asset}` to a discriminated
union admitting **multi-face** tokens (swappable by manual selection or condition) and
**animated** tokens (spritesheet/frame-driven), per the forward-looking token
architecture ([[token-architecture-forward-looking]]) seeded in M8d and locked in the
M10 parent spec §11. Realizes the Container-per-token render migration the parent spec
anticipated, which M10j (fx/emotes) will also depend on.

## 2. Constraints inherited

- **#5** Tokens render from the Zod `DocumentStore` via the reconciler; no client ECS.
- **#6** Server stays structural-only — `visual` is opaque `system`-body JSON, same as
  `movementModel`/`bounds`/`snapToGrid`. No ts-rs type, no server change.
- Token visuals are cosmetic — not a secrecy-gated surface. Standard two-reviewer
  review tier applies (not the movement/secrecy buddy-check tier), though buddy-check
  remains available on request.
- **`resolveTokenBox`/`resolveTokenActor` stay the single read-through** for size/shape/
  faction/conditions — this checkpoint adds a sibling resolver for visual only, it does
  not touch those.

## 3. Data model (client core — `scene-docs.ts` + `actor.ts`)

### 3.1 The `RenderVisual` / `FaceVisual` / `TokenVisual` types

```ts
// The two kinds the render layer actually draws — the render boundary.
type RenderVisual =
  | { kind: "image"; asset: string }
  | { kind: "animated"; source: AnimatedSource; fps: number; loop: boolean };

type AnimatedSource =
  | { type: "frames"; frames: string[] }                       // ordered asset UUIDs
  | { type: "sheet"; asset: string; rows: number; cols: number; count?: number };

// A face is itself a renderable visual — animated faces fall out for free, no nesting
// (a FaceVisual is never itself a "faces" kind).
type FaceVisual = RenderVisual;

// What actor-data / overrides may declare.
type TokenVisual =
  | RenderVisual
  | {
      kind: "faces";
      faces: Record<string, FaceVisual>;   // name -> face visual
      default: string;                     // name, must be a key of `faces`
      faceMap?: Record<string, string>;    // conditionId -> face name (optional, admitted now)
    };
```

- `TokenVisual` is the shape of `actor.system.visual` and `token.system.overrides.visual`
  (the existing whitelist entry — unchanged key, richer value).
- `FaceVisual = RenderVisual` is a deliberate non-nesting rule: a `faces` entry is never
  itself `{kind:"faces",...}`. Enforced by the TS type (structurally impossible) and
  asserted by a resolver test.
- No `generated` kind yet (M10i); no server type — this is the first purely-client M10
  checkpoint (mirrors `movementModel`/`snapToGrid`/`bounds` precedent for opaque,
  client-owned `system`-body axes).

### 3.2 Active face selection — per-token, not per-visual

The mutable **active face selection** lives on the token, not inside the `visual`
object — so N tokens linked to one actor's `faces` visual each hold independent
selections, mirroring how `conditions[]` already works per-token-vs-per-actor.

- New field: `token.system.face?: string` — token-local, always (present on both linked
  and instanced tokens; NOT part of the `overrides` whitelist, since it selects *into*
  the actor's faces map rather than overriding actor-data).
- Resolution precedence (highest first), computed only when the effective visual's kind
  is `"faces"`:
  1. `token.system.face`, if it names a key present in `faces`.
  2. The first key of `faceMap` (if present) whose condition id is in the token's
     effective `conditions[]` (via the existing `resolveConditions` order — first match
     wins, no severity ranking; documented as a v1 simplification, not a defect).
  3. `default`.
  4. If `default` is itself missing/invalid: the first key of `faces` in insertion
     order (fail-closed continuation, never a missing-visual `null`).
- If `faces` is empty (`{}` — a malformed actor edit): resolution fails closed to `null`
  (no visual renders), matching the existing `visual?.kind !== "image"` guard's current
  fail-closed behavior in `token-view.ts`.

### 3.3 `resolveTokenVisual` — the new read-through

New export in `src/client/core/src/actor.ts`, sibling to `resolveTokenActor`/
`resolveTokenBox`/`resolveConditions`:

```ts
function resolveTokenVisual(
  token: WireDocument,
  store: ReadableDocuments,
  eff?: EffectiveActor | null,
): RenderVisual | null
```

- Takes the already-resolved `EffectiveActor.visual` (or resolves it itself if `eff` is
  omitted, mirroring `resolveTokenBox`'s optional-`eff` convention) plus the token's own
  `conditions` (via `resolveConditions`) and `token.system.face`.
- `image` / `animated` kinds pass through unchanged (already a `RenderVisual`).
- `faces` kind resolves per §3.2 and returns the selected `FaceVisual` (itself a
  `RenderVisual` — image or animated).
- Unknown/malformed `kind`, or a `faces` visual with an empty `faces` map: returns
  `null`. Never throws.
- `AnimatedSource` is structurally validated at the boundary (`rows`/`cols` positive
  integers for `sheet`; non-empty `frames` array for `frames`) — an invalid source
  degrades to `null` (no render), not a partial/garbled sprite.

### 3.4 `TokenNodeSpec` update (`src/client/render/src/types.ts`)

Replaces the single `url: string` field with a discriminated `visual` field so the
backend can distinguish image vs animated without re-deriving it:

```ts
interface TokenNodeSpec {
  x: number; y: number; w: number; h: number; rotation: number;
  visual:
    | { kind: "image"; url: string }
    | { kind: "animated"; source: ResolvedAnimatedSource; fps: number; loop: boolean };
  borderColor: number | null;
  badges: string[];
  shape: "square" | "circle";
}

// Asset UUIDs already resolved to URLs by the AssetResolver at spec-build time
// (mirrors today's `assets.url(visual.asset)` call in token-view.ts).
type ResolvedAnimatedSource =
  | { type: "frames"; urls: string[] }
  | { type: "sheet"; url: string; rows: number; cols: number; count?: number };
```

`token-view.ts::toSpec` calls `resolveTokenVisual`, then maps its `asset`/`source` UUIDs
through `this.assets.url(...)` before constructing the spec — asset resolution stays a
`TokenView` concern (as it is today), never pushed into the render backend.

## 4. Render layer — Container-per-token migration

### 4.1 Structure

Each token becomes a Pixi `Container` (replacing today's bare `Sprite` + tracked
siblings):

```
tokenContainer            (position = center; rotation = 0)
├─ visualContainer        (rotation = token rotation)
│   ├─ Sprite | AnimatedSprite   (the resolved visual; swapped on kind change)
│   └─ border: Graphics          (footprint outline — rotates with the token)
└─ badges: Text[]         (children of tokenContainer, NOT visualContainer — stay
                            upright regardless of token rotation, matching today's
                            "badges track position, not rotation" behavior exactly)
```

- **Border and badges become real children** of the token's own Container instead of
  manually-repositioned sibling nodes tracked by id in parallel Maps
  (`tokenBorders`/`tokenBadges`/`tokenBadgeKeys` in `pixi-backend.ts`) — Pixi's own
  transform propagation replaces the per-tick `place()`/`.position.set(...)` calls for
  position and the separate `.angle` assignment for the border. This removes a whole
  class of "did I remember to reposition this sibling" bugs before M10j adds more
  per-token decorations (fx filters, emote overlays) on the same Container.
- `visualContainer` exists as a rotation boundary distinct from the outer
  `tokenContainer` specifically so **badges stay upright** while the border and image/
  animation rotate with the token — same visual outcome as today, cleaner mechanism.
- **`animated` visual**: a Pixi `AnimatedSprite` with `autoUpdate = false`; advanced
  explicitly by the existing `TokenView.tick(dtMs)` call (already wired for
  `TokenAnimator` tweening) via `sprite.currentTime += dtMs * spec.fps / 1000` (Pixi's
  own frame-advance model) each tick, respecting `loop` (clamp at last frame when
  `loop:false`) — no second ticker, no `PIXI.Ticker` auto-mode, deterministic and
  pausable exactly like the tween.
  - **`frames` source**: `Assets.load(url)` each configured URL → ordered `Texture[]`.
  - **`sheet` source**: load the one sheet texture, slice into `rows*cols` sub-`Texture`s
    via `Texture.frame` rects (`sheetTex.width/cols`, `sheetTex.height/rows`), truncated
    to `count` if given. Degenerate `rows`/`cols` (≤0, non-integer) already rejected at
    the resolver boundary (§3.3) — the backend only ever receives valid values.
- **Kind swap** (image↔animated, or an asset/source change): destroys and recreates only
  the visual child inside `visualContainer`; the outer Container, border, and badges
  persist across the swap (no full token teardown/recreate, preserving today's
  "transform-only re-push is cheap" property for tweening tokens).
- `TokenAnimator` and `TokenView.reconcile()`'s tween-target logic are **unchanged** —
  only the thing the resolved transform is applied to moves from a bare `Sprite` to
  `tokenContainer`.
- Hide/occlusion (`wasHidden` gap-entry/exit in `token-view.ts`) removes/recreates the
  whole `tokenContainer` — unchanged semantics, now a single `destroy({children:true})`
  instead of three separate Map-tracked destroys (sprite, border, badges).

### 4.2 What does NOT change

- `TokenAnimator`'s tween math, `animateAlongPath`/`animateSamples`, and the hide-gap
  detection in `TokenView.push()` are untouched.
- `resolveTokenBox`/`resolveTokenActor`/`resolveConditions`/faction-color resolution in
  `toSpec` are untouched — only the `visual`/`url` field construction changes.
- No change to `addLayerFilter` (still per-layer) — a per-token filter/overlay attach
  point on `tokenContainer` is real estate M10j will use, but M10h does not add the
  attach-point API itself (YAGNI: nothing in this checkpoint consumes it). The Container
  structure alone is what M10j needs; the filter API is that checkpoint's own scope.

## 5. Authoring UI (`src/modules/actors/ActorsPanel.svelte`)

- The existing single-image visual picker becomes a **kind editor** with three modes:
  - **Image** (unchanged control, existing behavior).
  - **Faces**: a repeatable list of `{name, visual}` rows — each row's `visual` is
    itself a nested image/animated mini-picker (reuses the same sub-controls as the
    animated editor below, not a separate implementation); a `default` name picker
    (constrained to existing row names); an optional repeatable `{conditionId, faceName}`
    row list for `faceMap` (condition ids drawn from the world condition registry,
    face names constrained to existing rows).
  - **Animated**: a source-type toggle (`frames` | `sheet`) — `frames` shows a
    repeatable ordered asset-UUID picker list; `sheet` shows one asset picker + numeric
    `rows`/`cols`/`count` inputs; shared `fps` (number) + `loop` (checkbox) controls.
- **Per-token face swap control**: a selection-driven palette (mirrors
  `module-conditions`' toggle-palette pattern exactly) listing the selected token's
  effective actor's face names; clicking one dispatches a `/system/face` update on the
  **token** doc (not the actor). Gated by `AppContext.canEdit(tokenDoc, "/system/face")`
  (GM or token owner) — same capability call condition-toggling already uses.
  - **Visible only when the resolved `TokenVisual.kind === "faces"`** — a plain
    image/animated token shows no face palette (nothing to swap).
  - **`old` field convention (load-bearing, per M10f-3's Critical fix):** the dispatched
    update reads the RAW stored `token.system.face ?? null` for `old`, never a
    resolved/defaulted value — matching the standing convention for every config-doc
    field-toggle editor in this codebase.

## 6. Testing

- **Core (`actor.test.ts` or new `resolveTokenVisual` suite):**
  - `image`/`animated` pass-through unchanged.
  - `faces` resolution precedence: manual `token.system.face` wins over `faceMap`; a
    valid `faceMap` condition match wins over `default`; `default` wins when neither
    matches; fail-closed final fallback (first key) when `default` itself is invalid;
    `null` when `faces` is empty.
  - An animated `FaceVisual` resolves correctly (proves the no-nesting, "face is itself
    a RenderVisual" design actually renders animated content, not just structurally
    permits it).
  - Malformed `AnimatedSource` (non-positive `rows`/`cols`, empty `frames`) → `null`.
- **Render (`token-view.test.ts` + a render-backend suite):**
  - Container structure: border + badges are children of the token Container (not
    tracked in separate id-keyed Maps); badges remain upright when the token rotates
    (angle applied to `visualContainer`, not the badge parent).
  - `AnimatedSprite` frame-advance is driven by `tick(dtMs)`, respects `fps`, clamps at
    the last frame when `loop:false`, wraps when `loop:true`.
  - Grid-sheet slicing produces `rows*cols` (or `count`-truncated) sub-textures of the
    expected frame rects.
  - Kind swap (image→animated and back) recreates only the visual child — border/badge
    node identity/reference is preserved across the swap (a regression guard for the
    "don't fully teardown on swap" design point in §4.1).
  - Existing image-token rendering is unchanged end-to-end (regression: today's tokens
    must render identically through the new Container/visual path).
  - GL-dependent assertions (actual pixel/texture checks) via Playwright per existing
    convention; pure structure/logic assertions via jsdom/unit tests.

## 7. Out of scope / deferred (this checkpoint)

- `generated` visual kind (parametric token generator) — M10i.
- Per-token fx filters + emotes, and the filter/overlay attach-point API on
  `tokenContainer` — M10j (the Container structure this checkpoint builds is what that
  checkpoint attaches to; the attach-point API itself is not built here).
- Faces-of-faces nesting — deliberately impossible by the `FaceVisual = RenderVisual`
  type (a face can never itself be `{kind:"faces"}`).
- Packed texture-atlas JSON (TexturePacker-style) as a third `AnimatedSource` — only
  `frames` (ordered asset list) and `sheet` (grid slice) ship; a richer atlas format can
  be added as a new `AnimatedSource` variant later without touching the resolver
  contract (`RenderVisual` stays the render boundary).
- Severity/priority ranking across multiple simultaneously-active `faceMap` conditions —
  first-match-in-`resolveConditions`-order wins; documented as a v1 simplification.
- No editing UI for changing an actor's `faces`/`animated` definition mid-session beyond
  the kind editor already covers (i.e. no drag-reorder for frame lists) — basic
  add/remove/edit rows only.

## 8. Reviewed skill-update gate targets

`shadowcat-codebase-scene-rendering` (Container-per-token structure, `AnimatedSprite`
tick-driven advance, `TokenNodeSpec.visual` discriminated shape) and
`shadowcat-codebase-actors-tokens` (`TokenVisual`/`FaceVisual`/`RenderVisual` types,
`resolveTokenVisual`, per-token `system.face` field, faces resolution precedence).

## 9. Review tier

Standard two-reviewer gate (`shadowcat-spec-reviewer` + `shadowcat-code-reviewer`) per
task. Not security/secrecy-sensitive (visuals are cosmetic; no new document-visibility
surface) — no mandatory whole-branch buddy-check, though available if the user wants
one, consistent with M10a-d's (non-movement) review tier.
