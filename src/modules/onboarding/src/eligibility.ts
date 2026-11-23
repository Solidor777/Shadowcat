import type { TourAudience, TourMeta } from "@shadowcat/core";
import type { AppContext } from "@shadowcat/ui-kit";
import type { TourProgress } from "./progress";

/** Who is looking at the screen: the facts a tour's `audience` and `scope` are matched against. */
export interface TourViewer {
  /** The current world's id. */
  world: string;
  /** The viewer's world-scoped role. */
  role: AppContext["role"];
  /** The viewer's server tier. */
  serverRole: AppContext["serverRole"];
}

/** Resolves a step's anchor id to a mounted element, or `null` when it is not on screen. The
 * engine passes `findTourAnchor` bound to `document`; tests pass a fake. */
export type AnchorResolver = (target: string) => HTMLElement | null;

/** Whether `audience` includes `viewer`. `"gm"` and `"player"` match exactly that world role (a
 * spectator matches neither), `"admin"` the server tier, `"any"` everyone.
 * @param audience The tour's declared audience.
 * @param viewer The current viewer.
 * @returns `true` when the tour is meant for this viewer.
 * @example
 * ```ts
 * import { matchesAudience } from "./eligibility";
 *
 * matchesAudience("gm", { world: "w1", role: "player", serverRole: "user" }); // false
 * ```
 */
export function matchesAudience(audience: TourAudience, viewer: TourViewer): boolean {
  switch (audience) {
    case "any":
      return true;
    case "gm":
      return viewer.role === "gm";
    case "player":
      return viewer.role === "player";
    case "admin":
      return viewer.serverRole === "admin";
  }
}

/** The indexes of `tour`'s steps whose target is mounted right now, in step order. A step whose
 * target is absent is skipped rather than failed, so a tour stays safe when a module is disabled
 * or a panel is closed.
 * @param tour The tour to resolve.
 * @param resolve Looks an anchor id up in the document.
 * @returns Indexes into `tour.steps`; empty when no step can be shown.
 * @example
 * ```ts
 * import type { TourMeta } from "@shadowcat/core";
 * import { resolvableSteps } from "./eligibility";
 *
 * const tour: TourMeta = {
 *   id: "example.tour",
 *   labelKey: "example.tour.name",
 *   version: 1,
 *   audience: "any",
 *   scope: "global",
 *   trigger: "first-use",
 *   steps: [{ target: "example.panel", titleKey: "t", bodyKey: "b", placement: "bottom" }],
 * };
 * resolvableSteps(tour, () => null); // []
 * ```
 */
export function resolvableSteps(tour: TourMeta, resolve: AnchorResolver): number[] {
  const shown: number[] = [];
  tour.steps.forEach((step, index) => {
    if (resolve(step.target) !== null) shown.push(index);
  });
  return shown;
}

/** The tours that should be offered automatically to `viewer`, in registration order: tours that
 * start on first use, whose audience includes the viewer, whose current version the viewer has
 * not finished, and that have at least one step that can be shown. Returns none when the viewer
 * switched tours off.
 * @param tours Every contributed tour.
 * @param viewer The current viewer.
 * @param progress The viewer's done-state.
 * @param resolve Looks an anchor id up in the document.
 * @returns The offerable tours.
 * @example
 * ```ts
 * import { offerableTours } from "./eligibility";
 * import { TourProgress } from "./progress";
 *
 * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
 * offerableTours([], { world: "w1", role: "gm", serverRole: "user" }, progress, () => null); // []
 * ```
 */
export function offerableTours(
  tours: readonly TourMeta[],
  viewer: TourViewer,
  progress: TourProgress,
  resolve: AnchorResolver,
): TourMeta[] {
  if (progress.isDisabled()) return [];
  return tours.filter(
    (tour) =>
      tour.trigger === "first-use" &&
      matchesAudience(tour.audience, viewer) &&
      progress.doneVersion(tour, viewer.world) < tour.version &&
      resolvableSteps(tour, resolve).length > 0,
  );
}
