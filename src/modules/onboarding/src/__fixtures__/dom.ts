import { findTourAnchor } from "@shadowcat/ui-kit";
import { TourController } from "../controller.svelte";
import { TourProgress } from "../progress";
import { GM, memorySettings, type ControllerHandle, type MemorySettings } from "./harness";
import type { TourMeta } from "@shadowcat/core";

/** Adds a tour anchor to the document the way `<Surface>` emits one: a `display: contents`
 * wrapper around a single element child.
 * @param id The anchor id.
 * @returns The element a spotlight should measure (the wrapper's child).
 * @example
 * ```ts
 * const el = mountAnchor("example.a");
 * ```
 */
export function mountAnchor(id: string): HTMLButtonElement {
  const wrapper = document.createElement("div");
  wrapper.setAttribute("data-tour", id);
  wrapper.style.display = "contents";
  const child = document.createElement("button");
  child.textContent = id;
  wrapper.append(child);
  document.body.append(wrapper);
  return child;
}

/** Builds a controller whose anchors are looked up in the real document through `findTourAnchor`.
 * @param tours The tours the registry holds.
 * @param settings The settings store; defaults to an empty one.
 * @returns The controller and its settings.
 * @example
 * ```ts
 * const { controller } = domController([]);
 * ```
 */
export function domController(
  tours: TourMeta[],
  settings: MemorySettings = memorySettings(),
): ControllerHandle {
  const controller = new TourController({
    progress: new TourProgress(settings, () => {}),
    serverEnabled: async () => true,
  });
  controller.attach({
    tours: () => tours,
    viewer: () => GM,
    resolve: (target) => findTourAnchor(document, target),
  });
  return { controller, settings };
}
