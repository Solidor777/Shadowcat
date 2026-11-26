// The `tour` contribution kind: plain-data guided-tour definitions a module contributes under
// `TOUR_CONTRACT`. Framework-neutral and DOM-free; the engine that offers and renders tours
// resolves each step's `target` against `data-tour` anchors (see `TOUR_ANCHOR_ATTR`).

import { z } from "zod";

/** Contract id modules contribute a guided tour under (`shadowcat.tour`, multi). Each
 * contribution carries a `tour` (`TourMeta`) and `component: null`; a tour engine reads them
 * through `ContributionRegistry.contributionsFor`. */
export const TOUR_CONTRACT = "shadowcat.tour";

/** The DOM attribute that names a tour anchor. `<Surface>` emits it for a contribution that sets
 * `Contribution.tourAnchor`, and any component may set it on its own root element. A step's
 * `target` is matched against this attribute's value, never against a CSS selector, so a tour
 * never reaches into another module's markup. */
export const TOUR_ANCHOR_ATTR = "data-tour";

/** Characters an anchor id or step target may use. The set is safe inside a double-quoted
 * attribute selector, so `[data-tour="<id>"]` needs no escaping, and it excludes every CSS
 * selector metacharacter. */
const ANCHOR_ID = /^[A-Za-z0-9._:-]+$/;

/** Whether `id` is a legal tour anchor id (non-empty, restricted character set).
 * @param id The candidate anchor id.
 * @returns `true` when `id` can name an anchor and be a step target.
 * @example
 * ```ts
 * import { isTourAnchorId } from "@shadowcat/core";
 *
 * isTourAnchorId("scene-tools.rail"); // true
 * isTourAnchorId(".rail > button"); // false
 * ```
 */
export function isTourAnchorId(id: string): boolean {
  return ANCHOR_ID.test(id);
}

/** Who a tour is for. `"gm"` and `"admin"` match the viewer's world role and server role
 * respectively; `"any"` matches everyone. */
export type TourAudience = "gm" | "player" | "admin" | "any";

/** Where a tour's done-state is stored: `"global"` once per account, `"world"` once per world. */
export type TourScope = "global" | "world";

/** How a tour is offered: `"first-use"` is offered automatically when no done entry exists for
 * its version; `"manual"` runs only when the user replays it. */
export type TourTrigger = "first-use" | "manual";

/** Which side of the target a step card sits on; `"center"` ignores the target's position. */
export type TourPlacement = "top" | "bottom" | "left" | "right" | "center";

/** One step of a tour. */
export interface TourStep {
  /** The `data-tour` anchor id this step highlights. A step whose anchor is not mounted is
   * skipped by the engine. */
  target: string;
  /** i18n key for the step's heading, resolved by the engine at render. */
  titleKey: string;
  /** i18n key for the step's body text, resolved by the engine at render. */
  bodyKey: string;
  /** Which side of the target the step card sits on. */
  placement: TourPlacement;
}

/** A guided tour: ordered steps plus the rules deciding who is offered it and when. */
export interface TourMeta {
  /** Namespaced tour id, e.g. `"core-ui.gm-start"`; the key under which done-state is stored. */
  id: string;
  /** i18n key for the tour's name, resolved by the engine where tours are listed (the replay
   * list in settings). */
  labelKey: string;
  /** Positive integer; bumping it re-offers the tour to users who finished an older version. */
  version: number;
  /** Who the tour is for. */
  audience: TourAudience;
  /** Where done-state is stored. */
  scope: TourScope;
  /** How the tour is offered. */
  trigger: TourTrigger;
  /** The ordered steps; at least one. */
  steps: TourStep[];
}

/** Zod schema for `TourStep`; strict, so an unknown key is rejected. */
export const TourStepSchema: z.ZodType<TourStep> = z
  .object({
    target: z.string().regex(ANCHOR_ID),
    titleKey: z.string().min(1),
    bodyKey: z.string().min(1),
    placement: z.enum(["top", "bottom", "left", "right", "center"]),
  })
  .strict();

/** Zod schema for `TourMeta`; strict, so an unknown key is rejected. */
export const TourMetaSchema: z.ZodType<TourMeta> = z
  .object({
    id: z.string().min(1),
    labelKey: z.string().min(1),
    version: z.number().int().positive(),
    audience: z.enum(["gm", "player", "admin", "any"]),
    scope: z.enum(["global", "world"]),
    trigger: z.enum(["first-use", "manual"]),
    steps: z.array(TourStepSchema).min(1),
  })
  .strict();

/** Parse and validate a tour definition.
 * @param meta The candidate `TourMeta`.
 * @returns The validated tour.
 * @throws When `meta` does not satisfy `TourMetaSchema`; the message names the failing path.
 * @example
 * ```ts
 * import { validateTourMeta } from "@shadowcat/core";
 *
 * const tour = validateTourMeta({
 *   id: "example.intro",
 *   labelKey: "example.tour.name",
 *   version: 1,
 *   audience: "any",
 *   scope: "global",
 *   trigger: "first-use",
 *   steps: [{ target: "example.panel", titleKey: "example.tour.title", bodyKey: "example.tour.body", placement: "bottom" }],
 * });
 * ```
 */
export function validateTourMeta(meta: unknown): TourMeta {
  const result = TourMetaSchema.safeParse(meta);
  if (!result.success) {
    const issue = result.error.issues[0];
    throw new Error(`invalid tour: ${issue.path.join(".") || "(root)"}: ${issue.message}`);
  }
  return result.data;
}
