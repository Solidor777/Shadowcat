import type { TourMeta } from "@shadowcat/core";
import { TourController, type TourEnvironment } from "../controller.svelte";
import type { TourViewer } from "../eligibility";
import { TourProgress, type ProgressSettings } from "../progress";

/** An in-memory account settings slice that records every patch it receives. */
export interface MemorySettings extends ProgressSettings {
  /** The patches `setUser` received, in order. */
  writes: Record<string, unknown>[];
  /** The slice as the server would hold it. */
  slice: Record<string, unknown>;
}

/** Builds an in-memory settings store; `setUser` merges like the server (a `null` removes a key).
 * @param initial The slice to start from.
 * @returns The store.
 * @example
 * ```ts
 * const settings = memorySettings({ disabled: true });
 * ```
 */
export function memorySettings(initial: Record<string, unknown> = {}): MemorySettings {
  const slice: Record<string, unknown> = { ...initial };
  const writes: Record<string, unknown>[] = [];
  return {
    writes,
    slice,
    getUser: () => slice,
    setUser: async (patch) => {
      writes.push(patch);
      for (const [key, value] of Object.entries(patch)) {
        if (value === null) delete slice[key];
        else slice[key] = value;
      }
    },
  };
}

/** A valid tour whose one step targets `anchor`; `over` overrides any field.
 * @param over Fields to override.
 * @returns The tour.
 * @example
 * ```ts
 * const t = tour({ id: "example.other", audience: "gm" });
 * ```
 */
export function tour(over: Partial<TourMeta> = {}): TourMeta {
  return {
    id: "example.intro",
    labelKey: "example.intro.name",
    version: 1,
    audience: "any",
    scope: "global",
    trigger: "first-use",
    steps: [{ target: "example.a", titleKey: "example.a.title", bodyKey: "example.a.body", placement: "bottom" }],
    ...over,
  };
}

/** A three-step tour over anchors `example.a`, `example.b` and `example.c`.
 * @param over Fields to override.
 * @returns The tour.
 * @example
 * ```ts
 * const t = threeStepTour();
 * ```
 */
export function threeStepTour(over: Partial<TourMeta> = {}): TourMeta {
  return tour({
    steps: ["a", "b", "c"].map((id) => ({
      target: `example.${id}`,
      titleKey: `example.${id}.title`,
      bodyKey: `example.${id}.body`,
      placement: "bottom" as const,
    })),
    ...over,
  });
}

/** A resolver that finds exactly the anchors named in `mounted`.
 * @param mounted The anchor ids that count as mounted.
 * @returns A resolver returning a detached element for a mounted id and `null` otherwise.
 * @example
 * ```ts
 * const resolve = mountedAnchors(["example.a"]);
 * ```
 */
export function mountedAnchors(mounted: readonly string[]): TourEnvironment["resolve"] {
  return (target) => (mounted.includes(target) ? document.createElement("div") : null);
}

/** The viewer a test runs as unless it says otherwise. */
export const GM: TourViewer = { world: "w1", role: "gm", serverRole: "user" };

/** A controller together with the settings store it writes to. */
export interface ControllerHandle {
  /** The controller under test. */
  controller: TourController;
  /** The store the controller's progress writes to. */
  settings: MemorySettings;
}

/** Builds a controller over `settings` and attaches an environment to it.
 * @param opts The options.
 * @param opts.tours Tours the registry holds.
 * @param opts.mounted Anchor ids that are mounted.
 * @param opts.viewer Viewer; defaults to a GM.
 * @param opts.settings Settings store; defaults to an empty one.
 * @param opts.serverEnabled Server switch; defaults to enabled.
 * @returns The controller and its settings.
 * @example
 * ```ts
 * const { controller } = makeController({ tours: [tour()], mounted: ["example.a"] });
 * ```
 */
export function makeController(opts: {
  /** Tours the registry holds. */
  tours: TourMeta[];
  /** Anchor ids that are mounted. */
  mounted: string[];
  /** Viewer; defaults to a GM. */
  viewer?: TourViewer;
  /** Settings store; defaults to an empty one. */
  settings?: MemorySettings;
  /** Server switch; defaults to enabled. */
  serverEnabled?: boolean;
}): ControllerHandle {
  const settings = opts.settings ?? memorySettings();
  const controller = new TourController({
    progress: new TourProgress(settings, () => {}),
    serverEnabled: async () => opts.serverEnabled ?? true,
  });
  controller.attach({
    tours: () => opts.tours,
    viewer: () => opts.viewer ?? GM,
    resolve: mountedAnchors(opts.mounted),
  });
  return { controller, settings };
}
