# dice-3d

## Purpose

A roll tumbles across the stage in 3D and lands showing exactly the result the server
rolled, on every recipient's screen. Contributes a transparent WebGL overlay canvas into
the stage's `STAGE_OVERLAY_CONTRACT` surface; the physics/rendering runtime (`three` +
`@dimforge/rapier3d-compat`) loads lazily on the first roll and never loads at all on a
device with 3D dice turned off.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `dice3d:overlay` | `shadowcat.stage-overlay` | `DiceOverlay` | — |

## Components

- `DiceOverlay.svelte` — the overlay canvas: mounts/resizes with the stage, subscribes to
  the document store directly for new rolls, and manages the tumble queue.
- `DiceEngine.ts` — the `three`/`rapier3d-compat` runtime: physics stepping, settle
  detection, and the label-texture remap that makes the up-face always show the server's
  result (`remapFaces.ts`).
- `geometry.ts` — the per-shape convex geometry (vertex cloud + oriented physical faces)
  shared by the visual mesh, the physics collider, and the up-face normal table.

## Contracts & seams

- **Provides** `shadowcat.stage-overlay` (multi cardinality).
- Subscribes directly to `AppContext.documents` for `message` documents carrying
  `roll_embed`/`table_draw` segments; no dependency on `chat-card`.
- `AppContext.dice3d.roll(outcome, rollId)`/`.clear()` — a late-binding bridge
  (`Dice3DBridge`) a system module can call even before the overlay has mounted.
- Reads `DieRecord.kind` (the server's authoritative face space) to pick which physical
  shape to render; a roll stored before this field existed renders no 3D dice for it.

## Pointers

- Source: `src/modules/dice-3d/`
- API: [`@shadowcat/module-dice-3d`](/api/ts/modules/_shadowcat_module-dice-3d.html)
