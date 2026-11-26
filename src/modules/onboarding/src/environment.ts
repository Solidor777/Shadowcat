import { TOUR_CONTRACT, type ContributionRegistry, type TourMeta } from "@shadowcat/core";
import { findTourAnchor, type AppContext } from "@shadowcat/ui-kit";
import type { TourEnvironment } from "./controller.svelte";
import type { TourViewer } from "./eligibility";

/** Every tour contributed under `TOUR_CONTRACT`, in registration order.
 * @param registry The app's contribution registry.
 * @returns The validated tour definitions.
 * @example
 * ```ts
 * import { ContributionRegistry } from "@shadowcat/core";
 * import { registeredTours } from "./environment";
 *
 * registeredTours(new ContributionRegistry()); // []
 * ```
 */
export function registeredTours(registry: ContributionRegistry): TourMeta[] {
  return registry.contributionsFor(TOUR_CONTRACT).flatMap((c) => (c.tour === undefined ? [] : [c.tour]));
}

/** The viewer described by an app context.
 * @param ctx The app context.
 * @returns The facts a tour's audience and scope are matched against.
 * @example
 * ```ts
 * import type { AppContext } from "@shadowcat/ui-kit";
 * import { viewerOf } from "./environment";
 *
 * declare const ctx: AppContext;
 * const viewer = viewerOf(ctx);
 * ```
 */
export function viewerOf(ctx: Pick<AppContext, "world" | "role" | "serverRole">): TourViewer {
  return { world: ctx.world, role: ctx.role, serverRole: ctx.serverRole };
}

/** The controller's view of the running app: the registry's tours, the viewer, and anchors found
 * in `root` through `findTourAnchor`, so no step target is ever a CSS selector.
 * @param ctx The app context.
 * @param root The node anchors are searched under; `document` in the running app.
 * @returns The environment to attach to a `TourController`.
 * @example
 * ```ts
 * import type { AppContext } from "@shadowcat/ui-kit";
 * import { appEnvironment } from "./environment";
 *
 * declare const ctx: AppContext;
 * const env = appEnvironment(ctx, document);
 * ```
 */
export function appEnvironment(ctx: AppContext, root: ParentNode): TourEnvironment {
  return {
    tours: () => registeredTours(ctx.contributions),
    viewer: () => viewerOf(ctx),
    resolve: (target) => findTourAnchor(root, target),
  };
}
