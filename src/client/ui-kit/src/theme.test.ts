import { describe, expect, it } from "vitest";
import {
  BUILTIN_THEMES,
  CONTRAST_PAIRINGS,
  DEFAULT_THEME_ID,
  THEME_TOKEN_NAMES,
  colorThemeTokenNames,
  contrastWarnings,
  themeIsolationCss,
  THEME_ISOLATION_CLASS,
  resolveTheme,
  sanitizeCustomTheme,
  sanitizeCustomThemes,
  wcagContrast,
  type ThemeTokenName,
} from "./theme";

describe("THEME_TOKEN_NAMES", () => {
  it("has no duplicates", () => {
    expect(new Set(THEME_TOKEN_NAMES).size).toBe(THEME_TOKEN_NAMES.length);
  });
});

describe("BUILTIN_THEMES", () => {
  it("starts with the default theme", () => {
    expect(BUILTIN_THEMES[0]!.id).toBe(DEFAULT_THEME_ID);
  });

  it("gives every built-in every token, with no extras", () => {
    const expected = new Set<string>(THEME_TOKEN_NAMES);
    for (const theme of BUILTIN_THEMES) {
      const keys = Object.keys(theme.tokens);
      expect(new Set(keys)).toEqual(expected);
    }
  });

  it("uses only literal token values (no var() chains)", () => {
    for (const theme of BUILTIN_THEMES) {
      for (const value of Object.values(theme.tokens)) {
        expect(value).not.toContain("var(");
      }
    }
  });

  it("has unique ids", () => {
    const ids = BUILTIN_THEMES.map((theme) => theme.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("resolveTheme", () => {
  it("returns the built-in theme for a built-in id", () => {
    expect(resolveTheme("slate-light", {}).id).toBe("slate-light");
  });

  it("falls back to the default theme for an unknown id", () => {
    expect(resolveTheme("no-such-theme", {}).id).toBe(DEFAULT_THEME_ID);
    expect(resolveTheme("", {}).id).toBe(DEFAULT_THEME_ID);
  });

  it("layers custom overrides onto the built-in base", () => {
    const resolved = resolveTheme("custom:mine", {
      mine: { label: "Mine", base: "slate-light", tokens: { accent: "#123456" } },
    });
    expect(resolved.id).toBe("custom:mine");
    expect(resolved.colorScheme).toBe("light");
    expect(resolved.tokens.accent).toBe("#123456");
    expect(resolved.tokens["surface-base"]).toBe(
      BUILTIN_THEMES.find((theme) => theme.id === "slate-light")!.tokens["surface-base"],
    );
  });

  it("uses the user label as the labelKey for a custom theme", () => {
    const resolved = resolveTheme("custom:mine", {
      mine: { label: "Mine", base: "slate-dark", tokens: {} },
    });
    expect(resolved.labelKey).toBe("Mine");
  });

  it("drops unknown token keys from custom overrides", () => {
    const resolved = resolveTheme("custom:mine", {
      mine: {
        label: "Mine",
        base: "slate-dark",
        tokens: { "not-a-token": "red" } as Partial<Record<ThemeTokenName, string>>,
      },
    });
    expect(resolved.tokens).not.toHaveProperty("not-a-token");
  });

  it("drops non-string override values", () => {
    const resolved = resolveTheme("custom:mine", {
      mine: {
        label: "Mine",
        base: "slate-dark",
        tokens: { accent: 42 } as unknown as Partial<Record<ThemeTokenName, string>>,
      },
    });
    expect(resolved.tokens.accent).toBe(
      BUILTIN_THEMES.find((theme) => theme.id === "slate-dark")!.tokens.accent,
    );
  });

  it("falls back to the default base for an unknown custom base", () => {
    const resolved = resolveTheme("custom:mine", {
      mine: { label: "Mine", base: "no-such-base", tokens: {} },
    });
    expect(resolved.colorScheme).toBe("dark");
    expect(resolved.tokens["surface-base"]).toBe(
      BUILTIN_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!.tokens["surface-base"],
    );
  });

  it("falls back to the default theme for an unknown custom id", () => {
    expect(resolveTheme("custom:missing", {}).id).toBe(DEFAULT_THEME_ID);
  });
});

describe("sanitizeCustomTheme", () => {
  it("keeps a well-formed theme", () => {
    expect(
      sanitizeCustomTheme({ label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } }),
    ).toEqual({ label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } });
  });

  it("drops unknown token keys", () => {
    const result = sanitizeCustomTheme({
      label: "Mine",
      base: "slate-dark",
      tokens: { accent: "#123456", bogus: "red" },
    });
    expect(result).toEqual({ label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } });
  });

  it("rejects garbage shapes", () => {
    expect(sanitizeCustomTheme(null)).toBeNull();
    expect(sanitizeCustomTheme("slate-dark")).toBeNull();
    expect(sanitizeCustomTheme({ base: "slate-dark", tokens: {} })).toBeNull();
    expect(sanitizeCustomTheme({ label: "Mine", base: "slate-dark", tokens: 7 })).toBeNull();
  });
});

describe("sanitizeCustomThemes", () => {
  it("keeps valid entries and drops garbage ones", () => {
    expect(
      sanitizeCustomThemes({
        good: { label: "Good", base: "slate-dark", tokens: {} },
        bad: "nope",
      }),
    ).toEqual({ good: { label: "Good", base: "slate-dark", tokens: {} } });
  });

  it("returns an empty map for non-object input", () => {
    expect(sanitizeCustomThemes(undefined)).toEqual({});
    expect(sanitizeCustomThemes("nope")).toEqual({});
  });
});

describe("colorThemeTokenNames", () => {
  it("includes only tokens every built-in gives an opaque color value", () => {
    const names = colorThemeTokenNames();
    expect(names).toContain("accent");
    expect(names).toContain("surface-base");
    expect(names).toContain("text-primary");
    // Translucency, shadow, stacking, and scale tokens are not color-input editable.
    expect(names).not.toContain("scrim");
    expect(names).not.toContain("drop-overlay");
    expect(names).not.toContain("shadow-elevated");
    expect(names).not.toContain("z-popover");
    expect(names).not.toContain("space-1");
    expect(names).not.toContain("font-sans");
  });

  it("pins every curated token to a #rrggbb value in every built-in", () => {
    // The theme editor feeds these values to `<input type="color">`, which
    // accepts only #rrggbb; a curated token drifting to another color syntax
    // must fail here rather than mis-render in the editor.
    for (const theme of BUILTIN_THEMES) {
      for (const name of colorThemeTokenNames()) {
        expect(theme.tokens[name]).toMatch(/^#[0-9a-f]{6}$/i);
      }
    }
  });

  it("returns tokens in THEME_TOKEN_NAMES order with no duplicates", () => {
    const names = colorThemeTokenNames();
    expect(new Set(names).size).toBe(names.length);
    expect(names).toEqual(THEME_TOKEN_NAMES.filter((name) => names.includes(name)));
  });
});

describe("wcagContrast", () => {
  it("is 21 for black on white and 1 for identical colors", () => {
    expect(wcagContrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(wcagContrast("#123456", "#123456")).toBe(1);
  });

  it("accepts 3-digit hex", () => {
    expect(wcagContrast("#000", "#fff")).toBeCloseTo(21, 5);
  });

  it("is symmetric", () => {
    expect(wcagContrast("#2d6ee8", "#ffffff")).toBeCloseTo(
      wcagContrast("#ffffff", "#2d6ee8"),
      10,
    );
  });
});

describe("CONTRAST_PAIRINGS", () => {
  it("references only real token names", () => {
    const names = new Set<string>(THEME_TOKEN_NAMES);
    for (const pairing of CONTRAST_PAIRINGS) {
      expect(names.has(pairing.fg)).toBe(true);
      expect(names.has(pairing.bg)).toBe(true);
    }
  });
});

describe("contrastWarnings", () => {
  it("reports no failures for any built-in theme", () => {
    for (const theme of BUILTIN_THEMES) {
      expect(contrastWarnings(theme.tokens)).toEqual([]);
    }
  });

  it("flags a pairing whose ratio falls below its minimum", () => {
    const tokens = {
      ...BUILTIN_THEMES[0]!.tokens,
      "text-primary": BUILTIN_THEMES[0]!.tokens["surface-base"],
    };
    const warnings = contrastWarnings(tokens);
    expect(
      warnings.some((w) => w.fg === "text-primary" && w.bg === "surface-base"),
    ).toBe(true);
  });
});

describe("themeIsolationCss", () => {
  it("re-declares every token at the default theme's value under the isolation class", () => {
    const css = themeIsolationCss();
    const defaults = BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!;
    expect(css.startsWith(`.${THEME_ISOLATION_CLASS} {`)).toBe(true);
    for (const name of THEME_TOKEN_NAMES) {
      expect(css).toContain(`--${name}: ${defaults.tokens[name]};`);
    }
    // Exactly one declaration per token — no more, no fewer: the emitted set
    // can never drift from the token universe in either direction.
    expect(css.match(/--[a-z0-9-]+:/g)).toHaveLength(THEME_TOKEN_NAMES.length);
    // `color-scheme` rides alongside: it inherits like a custom property, so
    // without it native controls inside the subtree would follow the user
    // theme's scheme while the tokens revert to the defaults.
    expect(css).toContain(`color-scheme: ${defaults.colorScheme};`);
  });
});
