// Executable WCAG 2.x contrast audit over every built-in theme: each pairing
// in `CONTRAST_PAIRINGS` (the single source the theme editor's warnings also
// read) must meet its minimum ratio against the theme DATA, so a palette
// regression fails a unit test, not a review.
import { describe, expect, it } from "vitest";
import { BUILTIN_THEMES, CONTRAST_PAIRINGS, wcagContrast } from "./theme";

describe("built-in theme contrast audit", () => {
  for (const theme of BUILTIN_THEMES) {
    describe(theme.id, () => {
      for (const pairing of CONTRAST_PAIRINGS) {
        it(`${pairing.fg} on ${pairing.bg} meets ${pairing.min}:1`, () => {
          expect(
            wcagContrast(theme.tokens[pairing.fg], theme.tokens[pairing.bg]),
          ).toBeGreaterThanOrEqual(pairing.min);
        });
      }
    });
  }
});
