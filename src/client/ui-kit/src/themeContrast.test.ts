// Executable WCAG 2.x contrast audit over every built-in theme's documented
// pairings. The helpers live here (no runtime dependency); the audit iterates
// the theme DATA, so a palette regression fails a unit test, not a review.
import { describe, expect, it } from "vitest";
import { BUILTIN_THEMES, type ThemeDefinition, type ThemeTokenName } from "./theme";

const SURFACE_TOKENS: ThemeTokenName[] = [
  "surface-sunken",
  "surface-base",
  "surface-raised",
  "surface-overlay",
];

/** Parses a 3- or 6-digit hex color into linear-light RGB channels. */
function parseHex(color: string): [number, number, number] {
  const hex = color.replace("#", "");
  const full =
    hex.length === 3
      ? hex.split("").map((ch) => ch + ch).join("")
      : hex;
  const channels: number[] = [];
  for (let i = 0; i < 3; i++) {
    const raw = parseInt(full.slice(i * 2, i * 2 + 2), 16) / 255;
    channels.push(raw <= 0.03928 ? raw / 12.92 : Math.pow((raw + 0.055) / 1.055, 2.4));
  }
  return channels as [number, number, number];
}

/** WCAG 2.x relative luminance of a hex color. */
function luminance(color: string): number {
  const [r, g, b] = parseHex(color);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.x contrast ratio between two hex colors (1–21). */
function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

function token(theme: ThemeDefinition, name: ThemeTokenName): string {
  return theme.tokens[name];
}

describe("built-in theme contrast audit", () => {
  for (const theme of BUILTIN_THEMES) {
    describe(theme.id, () => {
      it("text-primary is AA (>=4.5) on every surface", () => {
        for (const surface of SURFACE_TOKENS) {
          expect(contrast(token(theme, "text-primary"), token(theme, surface))).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("text-muted is AA (>=4.5) on the raised and overlay surfaces", () => {
        for (const surface of ["surface-raised", "surface-overlay"] as const) {
          expect(contrast(token(theme, "text-muted"), token(theme, surface))).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("on-accent is AA (>=4.5) on accent", () => {
        expect(contrast(token(theme, "on-accent"), token(theme, "accent"))).toBeGreaterThanOrEqual(4.5);
      });

      it("on-danger is AA (>=4.5) on danger", () => {
        expect(contrast(token(theme, "on-danger"), token(theme, "danger"))).toBeGreaterThanOrEqual(4.5);
      });

      it("danger is AA (>=4.5) on every surface (inline alert text)", () => {
        for (const surface of SURFACE_TOKENS) {
          expect(contrast(token(theme, "danger"), token(theme, surface))).toBeGreaterThanOrEqual(4.5);
        }
      });

      it("accent meets non-text contrast (>=3:1) against surface-base", () => {
        expect(contrast(token(theme, "accent"), token(theme, "surface-base"))).toBeGreaterThanOrEqual(3);
      });
    });
  }
});
