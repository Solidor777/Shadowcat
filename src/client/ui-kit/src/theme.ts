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
  },
};

/** The built-in themes, default first. */
export const BUILTIN_THEMES: readonly ThemeDefinition[] = [
  SLATE_DARK,
  SLATE_LIGHT,
  CONTRAST_DARK,
];

/** Keeps only entries whose key is a real token name and whose value is a
 * string — the validation every custom override set passes through. */
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
