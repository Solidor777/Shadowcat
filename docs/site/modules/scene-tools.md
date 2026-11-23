# scene-tools

## Purpose

The canvas tool rail: place / select / move / draw / template / measure / ping /
wall / region tools. Contributes into core-ui's toolrail surface and drives the
canvas exclusively through public seams — it never imports core-ui or render
internals (the contract-only element boundary).

## Contributions

| Id | Contract | Component | Meta |
|---|---|---|---|
| `scene-tools:toolrail` | `shadowcat.surface:toolrail` | `ToolRail` | — |
| `scene-tools:tour` | `shadowcat.tour` | none | tour `scene-tools.intro`, first use (anchors `scene-tools.rail` and one per tool) |

## Components

- `ToolRail.svelte` — tool selection + per-tool options.
- `AssetPicker.svelte` — background/art picker used by placement flows.
- `RegionTriggerTeleportEditor.svelte` — a region trigger row's `Teleport`
  effect editor: destination scene (live search; untouched = the region's own
  scene), x/y, pick-on-stage targeting, elevation, and a VFX asset picker.

## Contracts & seams

- **Requires** `shadowcat.surface:toolrail` (multi, from core-ui); depends on
  `core-ui ^0.1.0`.
- Drives `ctx.scene` (active tool, snap, drag), `ctx.actorSelection` (what the
  place tool stamps), `ctx.tokenSelection`, `ctx.sendPing`, and the
  `pathfind`/`moveRequest` seams for gated movement.
- **Elevation-band stamping**: `ToolContext.viewedLevelBand`/
  `viewedLevelBottom` (resolved by `ToolRail` from `ctx.viewedLevel` + the
  viewed scene's `levels`) stamp newly-authored geometry to the currently
  viewed floor — `viewedLevelBand` (a `{bottom,top}` pair) onto walls/
  regions/drawings/templates' `/engine/elevation`, `viewedLevelBottom` (a
  point value) onto newly-placed tokens/lights'. Absent/no viewed level ⇒
  `elevation: null` (today's behavior for a level-less scene, unchanged).
  `editWallElevation`'s band-edit logic is factored into a shared
  `editElevationBand` helper, reused by the region/drawing/template
  elevation-band editors (`editRegionElevation`/`editDrawingElevation`/
  `editTemplateElevation`): the select tool picks a region/drawing/template
  shape (`topRegionAt`/`topDrawingAt`/`topTemplateAt` in `hit-test.ts`, reusing
  the SAME tessellation `regionShapeSpec`/`drawingShapeSpec`/`templateShapeSpec`
  the corresponding render-layer view draws from) into
  `ToolController.editingEntity`, and `ToolRail` renders the matching
  elevation-band editor. Shape/fill/stroke/color/behavior/cost/secrecy/triggers
  remain create-only for all four kinds (region/drawing/template/wall) — a GM
  re-authors those by delete+recreate.
- **Sense-blocking regions**: the region tool carries three checkboxes
  (`ToolController.regionOccludesSight`/`regionOccludesLight`/
  `regionOccludesSound`) that persist as `RegionEngine.occludes` on the next
  region (none ticked persists `null`; the server rejects an all-false block).
  The region editor edits the same three flags on an existing region through
  `/engine/occludes`, writing `null` when the last one is cleared. The region
  layer draws each sense on its own channel (`regionShapeSpec`): sight swaps
  the fill for a pale haze, light darkens and thickens the rim, sound dashes
  the ring (`ShapeNodeSpec.stroke.dash`, painted by `PixiBackend` as one
  subpath per dash from `dashPath`). The server turns the covered-cell outline
  into walls for the blocked senses; movement is unaffected.
- **`Teleport` region trigger**: `ToolController.beginPickPortalTarget`/
  `endPickPortalTarget` capture one stage click as the trigger's destination
  x/y, temporarily roaming the viewed scene (`ToolContext.setGmViewedScene`,
  sourced from `AppContext.setGmViewedScene`) to the authored destination
  scene while picking, then restore both the original scene and the
  previously active tool.

## Pointers

- Source: `src/modules/scene-tools/`
- API: [`@shadowcat/module-scene-tools`](/api/ts/modules/_shadowcat_module-scene-tools.html)
