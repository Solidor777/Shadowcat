// DOM lookup for tour anchors. The tour engine and tests share this one resolver so an anchor
// emitted by `<Surface>` (a `display: contents` wrapper with no box of its own) and an anchor a
// component sets on its own root element resolve the same way.

import { TOUR_ANCHOR_ATTR, isTourAnchorId } from "@shadowcat/core";

/** Resolve a tour anchor id to the element whose bounding box a spotlight should measure.
 *
 * The first `[data-tour="<id>"]` match in document order wins. When that element is a
 * `display: contents` wrapper (what `<Surface>` emits) it has no box, so its first element child
 * is returned instead; a wrapper with no element child resolves to `null`. An unmounted anchor
 * also resolves to `null`, which the engine treats as "skip this step".
 * @param root The node to search under, usually `document`.
 * @param id The anchor id, as named by a tour step's `target`.
 * @returns The element to measure, or `null` when the anchor is absent or has no box.
 * @example
 * ```ts
 * import { findTourAnchor } from "./tourAnchors";
 *
 * const el = findTourAnchor(document, "scene-tools.rail");
 * ```
 */
export function findTourAnchor(root: ParentNode, id: string): HTMLElement | null {
  if (!isTourAnchorId(id)) return null;
  const el = root.querySelector<HTMLElement>(`[${TOUR_ANCHOR_ATTR}="${id}"]`);
  if (!el) return null;
  if (el.style.display === "contents") return el.firstElementChild instanceof HTMLElement ? el.firstElementChild : null;
  return el;
}
