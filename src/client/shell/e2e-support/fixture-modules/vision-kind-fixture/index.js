// Test-only installed module: registers the custom vision treatment `fixture-glow`.
// The Playwright loopback server discovers it through `SHADOWCAT_MODULES_DIR`; it is never part of
// the built client (`dist/`) or the release binary, and no world enables it unless a spec does.
// The bare `@shadowcat/core` specifier resolves through the shell's import map, the same way
// every installed module's runtime imports do.
import { VISION_TREATMENT_CONTRACT } from "@shadowcat/core";

/** The treatment name a `custom` render hint selects to reach this module's resolver. */
export const TREATMENT_KIND = "fixture-glow";

/** Packed `0xRRGGBB` color of every paint this treatment returns. */
export const GLOW_COLOR = 0x00ff88;

/**
 * Resolves a cell's paint from the hint's params: `{"paint":"outline"}` yields a stroke-only
 * outline, anything else a translucent fill. The two shapes feed the render layer's two separate
 * counters, so a spec can tell which branch ran from the painted frame.
 * @param {unknown} params The `custom` hint's opaque params payload.
 * @returns {import("@shadowcat/core").VisionTreatmentPaint} The paint for the cell.
 * @example
 * ```
 * resolveGlow({ paint: "outline" }); // { shape: "outline", color: 0x00ff88 }
 * ```
 */
export function resolveGlow(params) {
  if (params !== null && typeof params === "object" && params.paint === "outline") {
    return { shape: "outline", color: GLOW_COLOR };
  }
  return { shape: "fill", color: GLOW_COLOR, opacity: 0.4 };
}

export default {
  manifest: {
    id: "vision-kind-fixture",
    version: "1.0.0",
    dependencies: {},
    engines: { shadowcat: "*" },
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "vision-kind-fixture:treatment",
      contract: VISION_TREATMENT_CONTRACT,
      component: null,
      visionTreatment: { kind: TREATMENT_KIND, resolve: resolveGlow },
    });
  },
};
