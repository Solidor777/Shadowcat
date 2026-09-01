// Themes are data, not stylesheets. A `ThemeDefinition` maps every tier-1 and
// tier-2 token to a literal value; application writes inline styles on each
// themed document's root element, which beats both the shell stylesheet and any
// cloned stylesheets in secondary windows. The shell stylesheets keep declaring
// the default theme's values for no-JS first paint, and a parity test pins the
// stylesheet-declared token set equal to `THEME_TOKEN_NAMES`.

/** Every tier-1 and tier-2 theme token name, without the `--` prefix. This is
 * the single source of truth for the token universe: the shell stylesheets
 * declare defaults for exactly these names, and every `ThemeDefinition.tokens`
 * map covers exactly these names. */
export const THEME_TOKEN_NAMES = [
  // Tier 1 — raw primitives.
  "slate-950",
  "slate-900",
  "slate-850",
  "slate-800",
  "slate-700",
  "slate-600",
  "slate-400",
  "slate-50",
  "blue-600",
  "blue-500",
  "blue-400",
  "blue-300",
  "red-500",
  "green-500",
  "amber-500",
  "space-1",
  "space-2",
  "space-3",
  "space-4",
  "space-6",
  "space-8",
  "radius-1",
  "radius-2",
  "font-sans",
  "font-size-caption",
  "font-size-sm",
  "font-size-md",
  "input-height-coarse",
  // Tier 2 — semantic aliases.
  "surface-sunken",
  "surface-base",
  "surface-raised",
  "surface-overlay",
  "border",
  "grid-line",
  "text-primary",
  "text-muted",
  "accent",
  "accent-hover",
  "accent-active",
  "on-accent",
  "danger",
  "on-danger",
  "success",
  "warning",
  "shadow-elevated",
  "z-popover",
  "scrim",
  "drop-overlay",
] as const;

/** Union of every theme token name (a member of `THEME_TOKEN_NAMES`). */
export type ThemeTokenName = (typeof THEME_TOKEN_NAMES)[number];

/** A complete theme: every token resolved to a literal value, plus the
 * `color-scheme` the host applies to each themed document's root element. */
export interface ThemeDefinition {
  /** Built-in id (e.g. `slate-dark`), or `custom:<id>` for a resolved custom theme. */
  readonly id: string;
  /** Display-label source: an i18n key for built-ins; for a resolved custom
   * theme the user's own label (a key no catalog defines renders verbatim, so
   * the label passes through the same `t()` call site). */
  readonly labelKey: string;
  /** The `color-scheme` value applied alongside the tokens, so native
   * scrollbars and form controls match the theme. */
  readonly colorScheme: "dark" | "light";
  /** The full token map. Every value is a literal — never a `var()` chain — so
   * application writes fully-resolved values and never depends on cross-rule
   * cascade order. */
  readonly tokens: Readonly<Record<ThemeTokenName, string>>;
}

/** A user-defined theme: a display label plus token overrides on a built-in base. */
export interface CustomTheme {
  /** The user's display label. */
  label: string;
  /** Id of the built-in theme the overrides layer onto; an unknown id resolves
   * to the default theme as base. */
  base: string;
  /** Token overrides. Unknown keys are dropped at resolution, so a theme saved
   * against a newer token set degrades gracefully. */
  tokens: Partial<Record<ThemeTokenName, string>>;
}

/** The id of the built-in default theme — a visual no-op against the shell
 * stylesheets' declared defaults. */
export const DEFAULT_THEME_ID = "slate-dark";

const TOKEN_NAME_SET: ReadonlySet<string> = new Set(THEME_TOKEN_NAMES);

/** Scale tokens every built-in shares: themes differ in color, not in spacing,
 * radii, type scale, or stacking order. */
const SHARED_SCALE = {
  "space-1": "0.25rem",
  "space-2": "0.5rem",
  "space-3": "0.75rem",
  "space-4": "1rem",
  "space-6": "1.5rem",
  "space-8": "2rem",
  "radius-1": "4px",
  "radius-2": "8px",
  "font-sans": 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif',
  "font-size-caption": "0.75rem",
  "font-size-sm": "0.875rem",
  "font-size-md": "1rem",
  "input-height-coarse": "44px",
  "z-popover": "1000",
} as const;

// The default theme. Its color values are a literal transcription of the shell
// stylesheets' declared defaults, so applying it changes nothing on screen.
const SLATE_DARK: ThemeDefinition = {
  id: DEFAULT_THEME_ID,
  labelKey: "settings.theme.slateDark",
  colorScheme: "dark",
  tokens: {
    "slate-950": "#16161f",
    "slate-900": "#1f1f2c",
    "slate-850": "#262635",
    "slate-800": "#2c2f40",
    "slate-700": "#363645",
    "slate-600": "#434558",
    "slate-400": "#9698ae",
    "slate-50": "#e7e8f2",
    "blue-600": "#245ac0",
    "blue-500": "#2d6ee8",
    "blue-400": "#538aeb",
    "blue-300": "#7fa8f1",
    "red-500": "#f37287",
    "green-500": "#3fb089",
    "amber-500": "#e0a23b",
    ...SHARED_SCALE,
    "surface-sunken": "#16161f",
    "surface-base": "#1f1f2c",
    "surface-raised": "#262635",
    "surface-overlay": "#2c2f40",
    border: "#363645",
    "grid-line": "#363645",
    "text-primary": "#e7e8f2",
    "text-muted": "#9698ae",
    accent: "#2d6ee8",
    "accent-hover": "#538aeb",
    "accent-active": "#245ac0",
    "on-accent": "#ffffff",
    danger: "#f37287",
    "on-danger": "#16161f",
    success: "#3fb089",
    warning: "#e0a23b",
    "shadow-elevated": "0 4px 12px rgba(0, 0, 0, 0.35)",
    scrim: "rgba(0, 0, 0, 0.5)",
    "drop-overlay": "rgba(45, 110, 232, 0.25)",
  },
};

// Light variant: the surface ramp inverts (raised surfaces are lighter than the
// app backdrop) and text/accent/danger darken to keep the same AA pairings.
const SLATE_LIGHT: ThemeDefinition = {
  id: "slate-light",
  labelKey: "settings.theme.slateLight",
  colorScheme: "light",
  tokens: {
    "slate-950": "#17171f",
    "slate-900": "#262633",
    "slate-850": "#37374a",
    "slate-800": "#484860",
    "slate-700": "#50526a",
    "slate-600": "#7c7d94",
    "slate-400": "#a9abbe",
    "slate-50": "#f2f3f9",
    "blue-600": "#1d4aa1",
    "blue-500": "#245ac0",
    "blue-400": "#2d6ee8",
    "blue-300": "#538aeb",
    "red-500": "#b3193a",
    "green-500": "#1a7f5c",
    "amber-500": "#8f6407",
    ...SHARED_SCALE,
    "surface-sunken": "#d8dae6",
    "surface-base": "#e4e6f0",
    "surface-raised": "#f2f3f9",
    "surface-overlay": "#ffffff",
    border: "#b9bbcb",
    "grid-line": "#c6c8d8",
    "text-primary": "#17171f",
    "text-muted": "#50526a",
    accent: "#245ac0",
    "accent-hover": "#1d4aa1",
    "accent-active": "#163a80",
    "on-accent": "#ffffff",
    danger: "#b3193a",
    "on-danger": "#ffffff",
    success: "#1a7f5c",
    warning: "#8f6407",
    "shadow-elevated": "0 4px 12px rgba(23, 23, 31, 0.18)",
    scrim: "rgba(23, 23, 31, 0.45)",
    "drop-overlay": "rgba(36, 90, 192, 0.18)",
  },
};

// High-contrast dark variant: near-black surfaces, maximum-ratio text, and
// brightened accent/danger pairs that take dark `on-*` foregrounds.
const CONTRAST_DARK: ThemeDefinition = {
  id: "contrast-dark",
  labelKey: "settings.theme.contrastDark",
  colorScheme: "dark",
  tokens: {
    "slate-950": "#000000",
    "slate-900": "#0c0c12",
    "slate-850": "#16161e",
    "slate-800": "#20202c",
    "slate-700": "#4e5064",
    "slate-600": "#6a6c82",
    "slate-400": "#b6b8cc",
    "slate-50": "#ffffff",
    "blue-600": "#2d6ee8",
    "blue-500": "#538aeb",
    "blue-400": "#a3c1f6",
    "blue-300": "#7fa8f1",
    "red-500": "#ff8b9d",
    "green-500": "#5fd4a8",
    "amber-500": "#f2b64f",
    ...SHARED_SCALE,
    "surface-sunken": "#000000",
    "surface-base": "#0c0c12",
    "surface-raised": "#16161e",
    "surface-overlay": "#20202c",
    border: "#9a9cb0",
    "grid-line": "#4e5064",
    "text-primary": "#ffffff",
    "text-muted": "#b6b8cc",
    accent: "#7fa8f1",
    "accent-hover": "#a3c1f6",
    "accent-active": "#538aeb",
    "on-accent": "#000000",
    danger: "#ff8b9d",
    "on-danger": "#000000",
    success: "#5fd4a8",
    warning: "#f2b64f",
    "shadow-elevated": "0 4px 12px rgba(0, 0, 0, 0.7)",
    scrim: "rgba(0, 0, 0, 0.7)",
    "drop-overlay": "rgba(127, 168, 241, 0.3)",
  },
};

/** The built-in themes, default first. */
export const BUILTIN_THEMES: readonly ThemeDefinition[] = [
  SLATE_DARK,
  SLATE_LIGHT,
  CONTRAST_DARK,
];

/** Keeps only entries whose key is a real token name and whose value is a
 * string — the validation every custom override set passes through.
 * @param tokens The override map to validate.
 * @returns A map containing only the valid overrides.
 * @example
 * ```ts
 * // internal helper; not part of the public API
 * sanitizeTokenOverrides({ accent: "#123456" }); // { accent: "#123456" }
 * ```
 */
function sanitizeTokenOverrides(
  tokens: Partial<Record<ThemeTokenName, string>>,
): Partial<Record<ThemeTokenName, string>> {
  const clean: Partial<Record<ThemeTokenName, string>> = {};
  for (const [key, value] of Object.entries(tokens)) {
    if (TOKEN_NAME_SET.has(key) && typeof value === "string") {
      clean[key as ThemeTokenName] = value;
    }
  }
  return clean;
}

/** Validates an arbitrary persisted value as a `CustomTheme`, dropping unknown
 * token keys. Returns `null` when the value is not a usable custom theme at
 * all (non-object, missing label/base strings, or a non-object `tokens` map).
 * @param value The persisted value to validate.
 * @returns The sanitized custom theme, or `null` for garbage.
 * @example
 * ```ts
 * import { sanitizeCustomTheme } from "@shadowcat/ui-kit";
 *
 * sanitizeCustomTheme({ label: "Mine", base: "slate-dark", tokens: {} });
 * ```
 */
export function sanitizeCustomTheme(value: unknown): CustomTheme | null {
  if (typeof value !== "object" || value === null) return null;
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.label !== "string" || typeof candidate.base !== "string") return null;
  if (typeof candidate.tokens !== "object" || candidate.tokens === null) return null;
  return {
    label: candidate.label,
    base: candidate.base,
    tokens: sanitizeTokenOverrides(
      candidate.tokens as Partial<Record<ThemeTokenName, string>>,
    ),
  };
}

/** Validates a persisted custom-theme map entry by entry, dropping garbage.
 * @param value The persisted map to validate.
 * @returns A map containing only the entries that passed `sanitizeCustomTheme`.
 * @example
 * ```ts
 * import { sanitizeCustomThemes } from "@shadowcat/ui-kit";
 *
 * const custom = sanitizeCustomThemes({ mine: { label: "Mine", base: "slate-dark", tokens: {} } });
 * ```
 */
export function sanitizeCustomThemes(value: unknown): Record<string, CustomTheme> {
  const clean: Record<string, CustomTheme> = {};
  if (typeof value !== "object" || value === null) return clean;
  for (const [id, entry] of Object.entries(value)) {
    const theme = sanitizeCustomTheme(entry);
    if (theme) clean[id] = theme;
  }
  return clean;
}

/** Resolves an active-theme selector to a full `ThemeDefinition`. A built-in
 * id returns that theme; `custom:<id>` layers the custom theme's validated
 * overrides onto its built-in base (unknown base → the default theme as base);
 * anything unresolvable returns the default theme.
 * @param active The active selector: a built-in id or `custom:<id>`.
 * @param custom The saved custom themes, keyed by id.
 * @returns The resolved theme definition.
 * @example
 * ```ts
 * import { resolveTheme } from "@shadowcat/ui-kit";
 *
 * const theme = resolveTheme("slate-light", {});
 * ```
 */
export function resolveTheme(
  active: string,
  custom: Record<string, CustomTheme>,
): ThemeDefinition {
  const builtin = BUILTIN_THEMES.find((theme) => theme.id === active);
  if (builtin) return builtin;
  if (active.startsWith("custom:")) {
    const entry = custom[active.slice("custom:".length)];
    if (entry) {
      const base =
        BUILTIN_THEMES.find((theme) => theme.id === entry.base) ??
        BUILTIN_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
      return {
        id: active,
        labelKey: entry.label || base.labelKey,
        colorScheme: base.colorScheme,
        tokens: { ...base.tokens, ...sanitizeTokenOverrides(entry.tokens) },
      };
    }
  }
  return BUILTIN_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
}

/** Whether `value` is an opaque color literal: a `#rgb`/`#rrggbb` hex, or a
 * three-component `rgb()`/`hsl()` with no alpha channel. Alpha-bearing syntax
 * (`#rgba`, `#rrggbbaa`, `rgba()`, `hsla()`, and the modern `rgb(… / a)` form)
 * is rejected: an opaque color input cannot represent alpha, so editing such a
 * value through one would silently drop the alpha.
 * @param value The token value to classify.
 * @returns True when the value is an opaque color.
 * @example
 * ```ts
 * // internal helper; not part of the public API
 * isOpaqueColorValue("#2d6ee8"); // true
 * isOpaqueColorValue("rgba(0, 0, 0, 0.5)"); // false
 * ```
 */
function isOpaqueColorValue(value: string): boolean {
  const v = value.trim().toLowerCase();
  if (/^#[0-9a-f]{3}$/.test(v) || /^#[0-9a-f]{6}$/.test(v)) return true;
  if (/^#[0-9a-f]{4}$/.test(v) || /^#[0-9a-f]{8}$/.test(v)) return false;
  return (
    /^rgb\([^()/]+,[^()/]+,[^()/]+\)$/.test(v) || /^hsl\([^()/]+,[^()/]+,[^()/]+\)$/.test(v)
  );
}

/** The tokens a user theme may override through a color input: those whose
 * value is an opaque color in EVERY built-in theme. Translucency tokens
 * (`scrim`, `drop-overlay`), shadow, stacking, spacing, and font tokens are
 * excluded — they are not representable by `<input type="color">` and always
 * inherit the base. Derived from the built-in maps rather than declared
 * separately, so a new token or built-in can never drift the editor's row set
 * from the theme data.
 * @returns The editable color token names, in `THEME_TOKEN_NAMES` order.
 * @example
 * ```ts
 * import { colorThemeTokenNames } from "@shadowcat/ui-kit";
 *
 * colorThemeTokenNames().includes("accent"); // true
 * ```
 */
export function colorThemeTokenNames(): ThemeTokenName[] {
  return THEME_TOKEN_NAMES.filter((name) =>
    BUILTIN_THEMES.every((theme) => isOpaqueColorValue(theme.tokens[name])),
  );
}

/** Parses a 3- or 6-digit hex color into linear-light RGB channels (the WCAG
 * 2.x sRGB transfer function).
 * @param color The hex color (`#rgb` or `#rrggbb`).
 * @returns The linear-light `[r, g, b]` channels.
 * @example
 * ```ts
 * // internal helper; not part of the public API
 * hexToLinearRgb("#ffffff"); // [1, 1, 1]
 * ```
 */
function hexToLinearRgb(color: string): [number, number, number] {
  const hex = color.replace("#", "");
  const full = hex.length === 3 ? hex.split("").map((ch) => ch + ch).join("") : hex;
  const channels: number[] = [];
  for (let i = 0; i < 3; i++) {
    const raw = parseInt(full.slice(i * 2, i * 2 + 2), 16) / 255;
    channels.push(raw <= 0.03928 ? raw / 12.92 : Math.pow((raw + 0.055) / 1.055, 2.4));
  }
  return channels as [number, number, number];
}

/** The WCAG 2.x contrast ratio between two hex colors (`#rgb` or
 * `#rrggbb`), from 1 (identical) to 21 (black on white). Non-hex input yields
 * `NaN`, which compares false against every minimum — a garbage token value
 * never produces a spurious warning.
 * @param a The first color.
 * @param b The second color.
 * @returns The contrast ratio.
 * @example
 * ```ts
 * import { wcagContrast } from "@shadowcat/ui-kit";
 *
 * wcagContrast("#000000", "#ffffff"); // 21
 * ```
 */
export function wcagContrast(a: string, b: string): number {
  const luminance = (color: string): number => {
    const [r, g, b] = hexToLinearRgb(color);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** A documented foreground/background token pairing and the minimum WCAG
 * contrast ratio it must meet. The pairings are declared once here and drive
 * both the built-in themes' executable audit and the theme editor's
 * non-blocking warnings, so the two can never disagree about which pairings
 * matter. */
export interface ContrastPairing {
  /** The foreground (text or glyph) token of the pairing. */
  readonly fg: ThemeTokenName;
  /** The background (surface or fill) token the foreground sits on. */
  readonly bg: ThemeTokenName;
  /** The minimum acceptable WCAG contrast ratio (4.5 for AA text, 3 for
   * non-text UI components). */
  readonly min: number;
}

/** Every surface token text can sit on. */
const SURFACE_TOKEN_NAMES: readonly ThemeTokenName[] = [
  "surface-sunken",
  "surface-base",
  "surface-raised",
  "surface-overlay",
];

/** The documented contrast pairings: primary and muted text on surfaces,
 * on-fill tokens on their fills, inline alert text on surfaces, and the
 * accent's non-text minimum against the base surface. */
export const CONTRAST_PAIRINGS: readonly ContrastPairing[] = [
  ...SURFACE_TOKEN_NAMES.map(
    (bg): ContrastPairing => ({ fg: "text-primary", bg, min: 4.5 }),
  ),
  { fg: "text-muted", bg: "surface-raised", min: 4.5 },
  { fg: "text-muted", bg: "surface-overlay", min: 4.5 },
  { fg: "on-accent", bg: "accent", min: 4.5 },
  { fg: "on-danger", bg: "danger", min: 4.5 },
  ...SURFACE_TOKEN_NAMES.map((bg): ContrastPairing => ({ fg: "danger", bg, min: 4.5 })),
  { fg: "accent", bg: "surface-base", min: 3 },
];

/** Evaluates `CONTRAST_PAIRINGS` against a token map and returns the pairings
 * below their minimum ratio. Used by the theme editor for its non-blocking
 * warnings; the built-in themes' audit asserts this returns empty for each of
 * them.
 * @param tokens A full token map (a resolved theme's `tokens`).
 * @returns The failing pairings, in `CONTRAST_PAIRINGS` order.
 * @example
 * ```ts
 * import { BUILTIN_THEMES, contrastWarnings } from "@shadowcat/ui-kit";
 *
 * contrastWarnings(BUILTIN_THEMES[0].tokens); // []
 * ```
 */
export function contrastWarnings(
  tokens: Readonly<Record<ThemeTokenName, string>>,
): ContrastPairing[] {
  return CONTRAST_PAIRINGS.filter(
    (pairing) => wcagContrast(tokens[pairing.fg], tokens[pairing.bg]) < pairing.min,
  );
}

/** The CSS class marking a theme-isolated subtree (a contribution opted out of
 * the host theme via `Contribution.styling`). The generated sheet from
 * `themeIsolationCss` re-declares every token at its engine-default value
 * under this class. */
export const THEME_ISOLATION_CLASS = "sc-theme-isolate";

/** The `id` of the injected `<style>` element carrying the isolation sheet —
 * used by the theme controller's per-document install to stay idempotent. */
export const THEME_ISOLATION_SHEET_ID = "sc-theme-isolate-sheet";

/** Generates the theme-isolation stylesheet: one rule re-declaring EVERY
 * theme token at the default theme's value under the isolation class, so an
 * isolated subtree renders with engine defaults regardless of the active
 * (possibly user-authored) theme. `color-scheme` is re-declared alongside the
 * tokens — it inherits like a custom property, and without it native
 * scrollbars/form controls inside an isolated subtree would follow the user
 * theme's scheme while the tokens revert to the defaults. Derived from the
 * theme data itself — the property set can never drift from
 * `THEME_TOKEN_NAMES` (a test pins the emitted names equal to it).
 * @returns The stylesheet text.
 * @example
 * ```ts
 * import { themeIsolationCss, THEME_ISOLATION_CLASS } from "@shadowcat/ui-kit";
 *
 * themeIsolationCss().startsWith(`.${THEME_ISOLATION_CLASS}`); // true
 * ```
 */
export function themeIsolationCss(): string {
  const defaults = BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!;
  const lines = THEME_TOKEN_NAMES.map((name) => `  --${name}: ${defaults.tokens[name]};`);
  lines.push(`  color-scheme: ${defaults.colorScheme};`);
  return `.${THEME_ISOLATION_CLASS} {\n${lines.join("\n")}\n}\n`;
}
