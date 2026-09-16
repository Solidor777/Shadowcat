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
  session (documents in, scene frames in, interactions out). Renders
  `LevelSwitcher` (any-viewer floor picker) and, GM-only, a ghost-other-levels
  toggle.
- `LevelSwitcher.svelte` — floor picker for a scene with declared `levels`:
  renders nothing when a scene has none. Reads `AppContext.viewedLevel`,
  writes through `AppContext.setViewedLevel` (GM-persisted; a player's viewed
  level instead tracks their own token's elevation and ignores writes).

## Contracts & seams

- **Requires** `shadowcat.surface:stage` (from core-ui); depends on
  `core-ui ^0.1.0`.
- Renders the **optimistic** document view; consumes `viewedSceneId`,
  `viewedLevel`, scene-derived channels (vision/fog/lighting), `move_stream`
  playback, and the render-layer API. The canvas renders what the server lets
  this user see — fog/vision arrive pre-clipped.
- **Levels**: `AppContext.viewedLevel`/`setViewedLevel` name the currently
  viewed floor on the viewed scene (`SceneEngine.levels`); a `viewedLevel`
  change re-subscribes the `"vision"` channel (`RenderEngine.reapplyViewedLevel`)
  since the SERVER computes explored-fog per level, not just per scene. Band-
  shaped docs (wall/region/drawing/template) scope to the viewed level via
  `bandContains` at the level's own `bottom`; point-elevation docs (token/light)
  scope via `level_of`/`levelOf`.
- **Ghost-other-levels** (GM-only toggle, `data-testid="ghost-other-levels"`):
  when on, tokens on OTHER levels of the viewed scene still render, desaturated
  and faded (`TokenFx` `desaturate` + the new `alpha` entry, composed into the
  same `ColorMatrixFilter` as every other token art effect — no separate
  opacity mechanism).
- **Observability**: `data-level` (the viewed level id, or `""`) and
  `data-token-count` (the viewed scene's token count, scoped to the viewed
  level) are written wherever the stage's other read-only debug attributes
  are — on every applied derived (vision) frame and on every document-store
  commit — so a level switch or a token's elevation crossing a level boundary
  both update promptly.

## Pointers

- Source: `src/modules/stage/` (render engine: `src/client/render/`)
- API: [`@shadowcat/module-stage`](/api/ts/modules/_shadowcat_module-stage.html)
