# stage

## Purpose

The canvas stage: hosts the engine-owned PixiJS render surface (scenes, tokens,
walls, lighting, fog) inside core-ui's stage region. The stage component
attaches the render engine to AppContext's `scene` interaction seam; rendering
itself lives in `src/client/render`, not in this module.

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `stage:stage` | `shadowcat.surface:stage` | `Stage` | — |

## Components

- `Stage.svelte` — mounts the PixiJS canvas, binds the render engine to the
  session (documents in, scene frames in, interactions out).

## Contracts & seams

- **Requires** `shadowcat.surface:stage` (from core-ui); depends on
  `core-ui ^0.1.0`.
- Renders the **optimistic** document view; consumes `viewedSceneId`,
  scene-derived channels (vision/fog/lighting), `move_stream` playback, and the
  render-layer API. The canvas renders what the server lets this user see —
  fog/vision arrive pre-clipped.

## Pointers

- Source: `src/modules/stage/` (render engine: `src/client/render/`)
- API: [`@shadowcat/module-stage`](/api/ts/modules/_shadowcat_module-stage.html)
