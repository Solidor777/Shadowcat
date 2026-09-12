# M26 — 3D dice — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.5 the seam this milestone
> owns; §7 three + rapier; §9 D5, D6). Goal: a roll tumbles across the stage in 3D and lands
> showing exactly the result the server rolled, on every recipient's screen, and costs
> nothing on a device that turns it off.

## 1. Server — one field, mirrored by hand

`DieRecord.kind: Option<DieKind>` (`#[serde(default)]`), filled at the three `DieRecord`
construction sites in `dice::eval::groups` (`resolve_group`'s main map, the exploded/faces
push, `push_extra`) from the die's `RawDie.kind` — `dice::recalc::rederive` calls the same
`resolve_group`, so roll and recalc are covered by one edit. The face SPACE becomes an
every-recipient fact like `natural` already is (GM-only `raw` still holds the full log).
Absent (a roll stored before the field existed) ⇒ that roll renders no 3D dice, fail closed.
**The `dice` crate carries no ts-rs derives by invariant** (the client mirrors it by hand): so
`src/client/core/src/chat-docs.ts`'s `dieRecordSchemaImpl`/`DieRecordSchema`/`DieRecord`
gain `kind: WireDieKindSchema.nullish()` (reusing the `WireDieKind` schema `WireRawRoll`
already declares), and the `roll_embed` member of `ChatSegment` — an inline union literal
today — is extracted into a named exported `RollEmbedSegment` interface (the
`TableDrawSegment` precedent and the core skill's `Extract<>` rule). `RollTooltip` and the
chat card render nothing new.

`dice-settings` (`DiceSettingsEngine`, which IS ts-rs-exported) gains `sound: Option<Uuid>`
(appended, `#[serde(default)]`; `Uuid` like every other asset reference in `chat::mod`): the
dice-clatter one-shot asset, `None` = silent. `DiceSettingsEngine` has no `validate` today
and `normalize_engine`'s `"dice-settings"` arm is a bare `round_trip`; this milestone adds
`DiceSettingsEngine::validate` (today: nothing beyond serde — the asset id is a `Uuid` by
type) and wires it into the arm the way the `"channel-registry"` arm calls its own — so the
next field to need a check has a home. ts-rs regen for this struct only.

## 2. Client — `src/modules/dice-3d/` (`@shadowcat/module-dice-3d`)

### 2.1 Where it draws (D5)

A transparent `<canvas>` absolutely positioned over the stage, `pointer-events: none`, sized
to the stage element (ResizeObserver), owned by `DiceOverlay.svelte`. It is contributed into
`STAGE_OVERLAY_CONTRACT` (`shadowcat.stage-overlay`, cardinality multi) — a contract the
`stage` module renders as a `<Surface>` of absolutely-positioned children over its canvas.
If `Stage.svelte` has no such surface today, this milestone ADDS it (it is the "canvas
overlays" replaceability seam ARCHITECTURE §2 invariant 7 names). The three.js renderer is
created lazily on the first roll (`import("three")`, `import("@dimforge/rapier3d-compat")`
dynamic — the chunks never load for a device with `dice3d: false`) with `alpha: true,
antialias: ctx.performance.current.antialias, powerPreference: "low-power"`, and disposed
after 60 s idle (context released; re-created on the next roll).

### 2.2 Simulation + result matching (D6)

- Tray = the visible stage rect projected onto a ground plane at z = 0 with four invisible
  walls; camera orthographic-ish top-down with slight tilt (dice read from above).
- Per die a convex-hull rigid body (`rapier` `ColliderDesc.convexHull`) with the geometry for
  its shape: d4 tetra, d6 cube, d8 octa, d10 pentagonal trapezohedron, d12 dodeca, d20 icosa,
  d100 = two d10; any other `DieKind` with ≤ 20 faces (a d3, a d7, a `Faces` list) uses the
  standard shape whose face count is ≥ the kind's face count with unused faces blank; a kind
  with MORE than 20 faces (other than the d100 split) renders as a d20 body whose EVERY face
  carries the same label — the die's final `value` — so no face ever shows a wrong label
  (a "value chip"); so EVERY kind renders. The target face for the remap is derived from
  `DieRecord.value`: for `Numeric` the value itself, for `Faces` the index of `value` in the
  face list (never from `natural`, which a reroll/explode may have replaced).
- Face labels are drawn at runtime onto a canvas texture per die (numbers or `Face` labels /
  symbols), so custom face sets need no art. Colors from the theme's accent by default;
  per-device override (`localStorage` `shadowcat.dice3d`: `{ color, labelColor, material:
  "plastic" | "metal" | "glass" }`).
- RNG seeded from `roll_id` (mulberry32) ⇒ the SAME throw on every client for the same roll
  — every recipient sees the same tumble; the server's authority over the VALUE is what D6
  pins, not the animation.
- Throw: spawn along a tray edge with seeded velocity/angular velocity; step at a fixed 60 Hz
  substep decoupled from render; settle when every body's linear+angular speed < ε for 300 ms
  or at 4 s (then damped to rest). Then `remapFaces(die)`: read the up-face index from the
  final quaternion (max dot of face normals with +Z), and rotate the label assignment so the
  up face carries `DieRecord.value` (for d100 the tens/ones split). The remap is a pure
  function `remapFaces(upFaceIndex, faceCount, targetIndex) -> labelOrder` with its own tests.
  Only the texture is swapped after settling — no motion after the last frame moves.
- Dice fade out 2.5 s after settle; a click on the overlay (a single `pointer-events: auto`
  dismiss layer while dice are visible, so the stage is not blocked otherwise) clears them.
- Concurrency: a queue of rolls; ≤ 3 rolls tumble at once (the tray is shared); a roll with
  > 30 dice renders 30 + a "+N" badge on the chat card path (the card is the source of truth;
  the overlay is a courtesy).
- `reducedMotion` ⇒ dice appear already settled (one frame) and fade; `dice3d: false` ⇒
  `roll()` is a no-op (read each call).

### 2.3 Trigger

The dice-3d module subscribes to the document store directly; the chat-card module has no
dependency on it and no contract is introduced for the trigger. At module mount it seeds a
`seen: Set<roll_id>` from every `message` document already in the store (the cold-start
snapshot is seeded into `DocumentStore` before the socket connects, so "already in the
store at mount" IS history — no timestamp and no connect-time seam is needed). On each
subsequent `store.subscribe` notification it scans changed `message` documents for roll
outcomes not yet in `seen` and plays them: a `roll_embed` segment (`RollEmbedSegment`) AND a
`table_draw` segment (`TableDrawSegment` — its `outcome.records` carry the same `DieRecord`
shape, so a table draw's die tumbles too; `RollButton` clicks and `/roll` both produce fresh
`roll_embed`s and need nothing special). A recalculated roll (a `roll_embed` whose
`recalc_history` length grew) re-plays with the new values under a `(roll_id, recalc_count)`
key. `AppContext.dice3d: { roll(outcome: RollOutcome, rollId: string): void; clear(): void }`
exposes the same entry point for a system module that rolls outside chat.
Sound: `dice-settings.sound` ⇒ `ctx.audio.playOneShot(sound, { channel: "sfx" })` at throw
time — wired in the integration task after M23 merges.

## 3. Tests

- Server: `DieRecord.kind` populated on roll and recalc (all three construction sites);
  serde default on an old stored segment; `dice-settings.sound` round-trips as a `Uuid` and
  the new `validate` is wired into `normalize_engine`.
- Core: `DieRecordSchema` accepts and types `kind`; `RollEmbedSegment` is a named export.
- Module (three + rapier mocked with `vi.mock` — no WebGL in jsdom): `remapFaces` truth table
  (every standard shape, d100 split, symbolic faces, unused-face padding); seeded throw
  determinism (same `roll_id` ⇒ same initial velocities); queue cap and > 30 dice badge;
  store-subscription trigger (a message present at mount never plays; a message arriving
  after mount plays once; a `table_draw` plays; a recalc re-plays; `dice3d:false` no-op;
  reduced motion settles instantly); overlay contract registration;
  lazy import happens on first roll only; idle disposal timer.
- e2e `dice-3d.spec.ts` (written here; dispatcher-run; inherits `playwright.config.ts`'s
  `--use-gl=angle` launch flag — the platform GL backend, SwiftShader only where no device
  exists; the spec assumes nothing about software rendering):
  GM `/roll 1d20` → both GM and player overlays reach `data-dice3d-state="settled"` within
  8 s and the settled value attribute `data-dice3d-values` equals the chat card's total;
  player switches `dice3d` off → next roll leaves the player's overlay `idle`.

## 4. Docs + skills

- `docs/site/modules/dice-3d.md`; `stage.md` documents `STAGE_OVERLAY_CONTRACT`; ARCHITECTURE
  §3 rows (three, rapier) and §4's 3D-dice row rewritten as built with the D5 decision.
- New skill `shadowcat-codebase-dice-3d` (master §6); updates to `dice` (`DieRecord.kind`)
  and `chat`; hook globs `src/modules/dice-3d/`.
- `docs/HISTORY.md` M26 entry.
