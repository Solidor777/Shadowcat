// Pins the two token declaration sites together: the stylesheet default-theme
// declarations and the theme data module's `THEME_TOKEN_NAMES`/`slate-dark`
// map. Both the NAME SET and the DEFAULT VALUES must agree, so a token added,
// renamed, or revalued on only one side fails here.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { BUILTIN_THEMES, DEFAULT_THEME_ID, THEME_TOKEN_NAMES } from "@shadowcat/ui-kit";

/** Reads one stylesheet and returns its `--name: value` declarations with the
 * prefix stripped and `//` comments removed. */
function readDeclarations(path: string): Map<string, string> {
  const text = readFileSync(fileURLToPath(path), "utf8").replace(/\/\/[^\n]*/g, "");
  const declarations = new Map<string, string>();
  for (const match of text.matchAll(/^\s*--([a-z0-9-]+)\s*:\s*([^;]+);/gm)) {
    declarations.set(match[1]!, match[2]!.trim());
  }
  return declarations;
}

const stylesheets = ["./_primitives.scss", "./_semantic.scss"].map(
  (name) => new URL(name, import.meta.url).href,
);

const declared = new Map<string, string>();
for (const path of stylesheets) {
  for (const [name, value] of readDeclarations(path)) declared.set(name, value);
}

/** Resolves `var(--x)` chains against the declared map, so stylesheet aliases
 * can be compared against the theme data's literal values. */
function resolveValue(name: string): string {
  let value = declared.get(name)!;
  while (value.includes("var(")) {
    value = value.replace(/var\(--([a-z0-9-]+)\)/g, (_, ref: string) => {
      const target = declared.get(ref);
      if (target === undefined) throw new Error(`unresolved token reference: ${ref}`);
      return target;
    });
  }
  return value.trim();
}

describe("theme token parity (stylesheets ≡ theme data)", () => {
  it("declares exactly the ThemeTokenName universe", () => {
    expect([...declared.keys()].sort()).toEqual([...THEME_TOKEN_NAMES].sort());
  });

  it("declares no token twice across the stylesheets", () => {
    const seen = new Set<string>();
    for (const path of stylesheets) {
      for (const name of readDeclarations(path).keys()) {
        expect(seen.has(name)).toBe(false);
        seen.add(name);
      }
    }
  });

  it("the default theme's literals equal the stylesheet defaults", () => {
    const slateDark = BUILTIN_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
    for (const name of THEME_TOKEN_NAMES) {
      expect(slateDark.tokens[name]).toBe(resolveValue(name));
    }
  });
});
