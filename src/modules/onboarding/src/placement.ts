import type { TourPlacement } from "@shadowcat/core";

/** A rectangle in viewport pixels. */
export interface Box {
  /** Distance from the viewport's left edge. */
  left: number;
  /** Distance from the viewport's top edge. */
  top: number;
  /** Width. */
  width: number;
  /** Height. */
  height: number;
}

/** Gap in pixels between a spotlighted target and its step card. */
export const CARD_GAP = 12;

/** Smallest distance in pixels the step card keeps from every viewport edge. */
export const VIEWPORT_MARGIN = 8;

/** Clamps `value` into `[min, max]`, favouring `min` when the range is empty.
 * @param value The value to clamp.
 * @param min The lower bound.
 * @param max The upper bound.
 * @returns The clamped value.
 * @example
 * ```ts
 * // private helper of `placeCard`
 * ```
 */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(value, max));
}

/** Where the step card goes: on the requested side of the target, then shifted back inside the
 * viewport. With no target, or `"center"`, the card is centred. When the requested side has no
 * room the card still ends up fully on screen, overlapping the target rather than leaving the
 * viewport.
 * @param target The target's box, or `null` when the step has none on screen.
 * @param placement The side the step asks for.
 * @param card The card's measured size.
 * @param viewport The viewport size.
 * @returns The card's top-left corner in viewport pixels.
 * @example
 * ```ts
 * import { placeCard } from "./placement";
 *
 * const corner = placeCard(
 *   { left: 100, top: 100, width: 40, height: 40 },
 *   "bottom",
 *   { width: 200, height: 80 },
 *   { width: 1000, height: 800 },
 * );
 * // corner.top === 152
 * ```
 */
export function placeCard(
  target: Box | null,
  placement: TourPlacement,
  card: Pick<Box, "width" | "height">,
  viewport: Pick<Box, "width" | "height">,
): Pick<Box, "left" | "top"> {
  const centred = {
    left: (viewport.width - card.width) / 2,
    top: (viewport.height - card.height) / 2,
  };
  let spot = centred;
  if (target !== null && placement !== "center") {
    const midX = target.left + target.width / 2 - card.width / 2;
    const midY = target.top + target.height / 2 - card.height / 2;
    switch (placement) {
      case "bottom":
        spot = { left: midX, top: target.top + target.height + CARD_GAP };
        break;
      case "top":
        spot = { left: midX, top: target.top - CARD_GAP - card.height };
        break;
      case "left":
        spot = { left: target.left - CARD_GAP - card.width, top: midY };
        break;
      case "right":
        spot = { left: target.left + target.width + CARD_GAP, top: midY };
        break;
    }
  }
  return {
    left: clamp(spot.left, VIEWPORT_MARGIN, viewport.width - card.width - VIEWPORT_MARGIN),
    top: clamp(spot.top, VIEWPORT_MARGIN, viewport.height - card.height - VIEWPORT_MARGIN),
  };
}
