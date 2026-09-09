import { expect } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

/** A point in stage-canvas-local pixels (the canvas's own top-left is the origin). At the
 * default camera these are also scene units, which is why a spec authors geometry in them. */
export interface ScenePoint {
  /** Distance right of the canvas's left edge. */
  x: number;
  /** Distance below the canvas's top edge. */
  y: number;
}

/** The stage canvas element.
 * @param page - The page whose stage to address.
 * @returns The canvas locator.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await stageCanvas(page).click({ position: { x: 10, y: 10 } });
 * ```
 */
export function stageCanvas(page: Page): Locator {
  return page.getByTestId("stage-canvas");
}

/** Clicks a scene coordinate on the stage canvas.
 *
 * Addresses the canvas as an ELEMENT rather than converting to page coordinates, so the gesture
 * inherits the actionability wait: the canvas must be visible, hold a bounding box unchanged
 * across consecutive frames, and be the element that receives the event. Opening or closing a
 * docked panel resizes the canvas, and a coordinate computed from a box read before that resize
 * settles lands somewhere else — on a host slow enough to finish the relayout first the gesture
 * happens to land correctly, so the defect is invisible exactly where a software-rendered suite
 * runs and surfaces the moment rendering gets faster.
 * @param page - The page whose stage is clicked.
 * @param at - The point in canvas-local coordinates.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await clickScene(page, { x: 210, y: 310 });
 * ```
 */
export async function clickScene(page: Page, at: ScenePoint): Promise<void> {
  await stageCanvas(page).click({ position: { x: at.x, y: at.y } });
}

/** Double-clicks a scene coordinate on the stage canvas. Element-relative for the same reason as
 * `clickScene` — see its doc.
 * @param page - The page whose stage is double-clicked.
 * @param at - The point in canvas-local coordinates.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await dblclickScene(page, { x: 610, y: 310 });
 * ```
 */
export async function dblclickScene(page: Page, at: ScenePoint): Promise<void> {
  await stageCanvas(page).dblclick({ position: { x: at.x, y: at.y } });
}

/** Hovers `anchor` as an ELEMENT-relative gesture, then returns the canvas's settled page-space
 * origin.
 *
 * Playwright's pointer primitives (`Mouse.move`, `Mouse.down`, `Mouse.up`) take page coordinates
 * and carry no actionability wait, so a drag or a press-and-poll cannot be expressed
 * element-relatively end to end and must convert. The hover is what makes the conversion sound:
 * it waits for the canvas to be visible, stable across consecutive frames, and hit-testable, so
 * the box read after it describes a settled layout. It also leaves the pointer exactly where the
 * gesture starts, which is where a press wants it.
 *
 * INVARIANT: the origin is only reachable through this function, so no caller can convert
 * canvas-local coordinates from a box read before the layout settled — which is the defect the
 * hover exists to prevent, and which no assertion downstream can detect (the gesture lands on
 * whatever the stale offset points at, silently doing nothing).
 * @param page - The page whose stage to measure.
 * @param anchor - Where the pointer should rest, in canvas-local coordinates.
 * @returns The canvas's top-left in page coordinates.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const origin = await sceneOrigin(page, { x: 100, y: 100 });
 * await page.mouse.down();
 * ```
 */
export async function sceneOrigin(page: Page, anchor: ScenePoint): Promise<ScenePoint> {
  const canvas = stageCanvas(page);
  await canvas.hover({ position: { x: anchor.x, y: anchor.y } });
  const box = await canvas.boundingBox();
  expect(box, "the stage canvas must be laid out before a pointer gesture").not.toBeNull();
  return { x: box!.x, y: box!.y };
}

/** The canvas's own center in canvas-local coordinates, measured after the layout settles.
 * @param page - The page whose stage to measure.
 * @returns The center point, in canvas-local coordinates.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await clickScene(page, await sceneCenter(page));
 * ```
 */
export async function sceneCenter(page: Page): Promise<ScenePoint> {
  const canvas = stageCanvas(page);
  // Hovering with no position targets the element's own centre, which is the point being
  // measured — and it is an ACTION, so it carries the same actionability wait `sceneOrigin`
  // relies on. A visibility assertion alone would not: it admits a box still resizing, and a
  // half-width canvas yields a centre that is not the centre, which is the defect this module
  // exists to close rather than a weaker version of it.
  await canvas.hover();
  const box = await canvas.boundingBox();
  expect(box, "the stage canvas must be laid out before it can be measured").not.toBeNull();
  return { x: box!.width / 2, y: box!.height / 2 };
}

/** Presses at `from`, moves through `waypoints` in order, and releases at the last one.
 *
 * A single waypoint is one pointermove for the whole displacement, which is what a segment-per-drag
 * tool (walls) authors; several produce a path, which is what a freehand tool samples.
 * @param page - The page whose stage is dragged.
 * @param from - The press point, in canvas-local coordinates.
 * @param waypoints - The move targets in order, in canvas-local coordinates; must be non-empty.
 * @param steps - Intermediate pointermove events synthesized per waypoint; one by default.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await dragScenePath(page, { x: 10, y: 10 }, [{ x: 50, y: 10 }, { x: 90, y: 40 }], 3);
 * ```
 */
export async function dragScenePath(
  page: Page,
  from: ScenePoint,
  waypoints: readonly ScenePoint[],
  steps = 1,
): Promise<void> {
  expect(waypoints.length, "a drag needs at least one destination").toBeGreaterThan(0);
  const o = await sceneOrigin(page, from);
  await page.mouse.down();
  for (const w of waypoints) await page.mouse.move(o.x + w.x, o.y + w.y, { steps });
  await page.mouse.up();
}

/** Presses at `from`, moves once to `to`, and releases — the single-displacement case of
 * `dragScenePath`.
 * @param page - The page whose stage is dragged.
 * @param from - The press point, in canvas-local coordinates.
 * @param to - The release point, in canvas-local coordinates.
 * @param steps - Intermediate pointermove events synthesized; one by default.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await dragScene(page, { x: 210, y: 310 }, { x: 410, y: 310 });
 * ```
 */
export async function dragScene(
  page: Page,
  from: ScenePoint,
  to: ScenePoint,
  steps = 1,
): Promise<void> {
  await dragScenePath(page, from, [to], steps);
}
