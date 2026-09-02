# Open Bugs

Currently open, confirmed-real defects. Deferrals belong in `TODO.md`, not here.

## Stage canvas mis-sizes after a world leave/re-enter cycle, breaking raw-coordinate pointer gestures

After a GM leaves a world (`Settings` panel's "leave world") and re-enters the same world, the
`.stage-host` canvas's bounding box can end up grossly larger than the viewport and vertically
offset (observed: `{x:48, y:-756, width:616, height:1444}` against a 720px-tall viewport,
immediately after re-entry, growing further after subsequent panel-open interactions). The DOM
ancestry from the canvas up through `dv-content-container`/`dv-groupview`/`dv-view-container`/
`dv-grid-view.dv-dockview` all report the identical oversized rect, with `dv-grid-view.dv-dockview`
carrying an explicit inline `height`/`width` style — the size originates from dockview's own root
sizing, not from CSS clipping further down.

Consequence: any pointer gesture computed from a `boundingBox()`/`getBoundingClientRect()` snapshot
taken shortly after re-entry and then used as raw page coordinates (`page.mouse.click(x, y)`) can
land off the actual rendered canvas entirely — `document.elementFromPoint` at the computed center
returns nothing. A `locator.click({ position })` call (which re-measures and auto-scrolls at click
time) does not exhibit the failure, which is why this surfaces specifically in gestures built on a
cached raw offset. It does not self-correct: waiting several seconds and firing a mouse move after
re-entry does not change the measured box.

Reproduced with a plain square-grid world (no hex grid involved) via: create world → leave world →
re-enter the same world → open the asset browser, upload an asset, activate the place tool, select
the asset → click the canvas. `data-token-count` never advances because the click coordinates never
reach the canvas's own pointer handlers (`onPointerDown` is never invoked).

Confirmed pre-existing: none of the files implicated in reproduction so far
(`src/modules/panels/src/engine/dockview.ts`, `src/modules/panels/src/PanelHost.svelte`,
`src/modules/stage/src/Stage.svelte`'s viewport measurement/`ResizeObserver` wiring,
`src/modules/core-ui/src/Layout.svelte`) were touched by any commit on `m18-token-enrichment`
(verified via `git log origin/main..HEAD` against each path) — this is not a regression introduced
by that branch's own work.

Affected: `src/client/shell/e2e/hex-movement.spec.ts`'s "a non-GM player's wall-crossing drag on a
hex scene is rejected by the server and rolled back" test fails deterministically (reproduced
across 3 consecutive retries) at the GM's own token-placement step, which follows a leave/re-enter
cycle earlier in the same test, using raw `page.mouse.click(origin.x + PLACE_X, origin.y +
TOKEN_Y)` coordinates from a `canvasOrigin()` helper. Not yet root-caused to a single fix site
within `PanelHost.svelte`'s `eng.init()`/adoption-effect sequencing or dockview-core's own resize
timing; needs an agent with the client-shell/panels subsystem context to pin the exact ordering
race and correct it (likely: ensure the stage element's dockview adoption completes, and a layout
reflow settles, before `Stage.svelte` takes its first `setViewport` measurement, or force a
re-measurement once adoption is confirmed complete rather than relying solely on the existing
`ResizeObserver`).
