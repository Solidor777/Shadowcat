import { describe, expect, it } from "vitest";
import {
  BUILTIN_THEMES,
  DEFAULT_THEME_ID,
  THEME_TOKEN_NAMES,
  resolveTheme,
  sanitizeCustomTheme,
  sanitizeCustomThemes,
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
