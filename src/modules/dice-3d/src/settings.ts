/** Per-device 3D-dice appearance override, persisted in `localStorage` (the theme mirror's
 * `readThemeMirror`/`writeThemeMirror` precedent). Empty `color`/`labelColor` mean "use the
 * active theme's accent" — resolved by the caller, not stored here. */
export interface Dice3DDeviceSettings {
  /** Die-body color override, a css `#rrggbb` string, or `""` to use the theme accent. */
  color: string;
  /** Face-label color override, a css `#rrggbb` string, or `""` to use the theme's primary text. */
  labelColor: string;
  /** Surface material preset. */
  material: "plastic" | "metal" | "glass";
}

const STORAGE_KEY = "shadowcat.dice3d";

const DEFAULTS: Dice3DDeviceSettings = { color: "", labelColor: "", material: "plastic" };

/**
 * Reads the per-device 3D-dice appearance override from `localStorage`. Fail-closed: a
 * missing key, unparseable JSON, or a garbage `material` value all fall back to `DEFAULTS`
 * rather than throwing.
 * @returns The persisted settings, or `DEFAULTS` if none are stored or storage is unavailable.
 * @example
 * ```ts
 * import { readDice3DSettings } from "@shadowcat/module-dice-3d";
 *
 * readDice3DSettings();
 * ```
 */
export function readDice3DSettings(): Dice3DDeviceSettings {
  if (typeof localStorage === "undefined") return DEFAULTS;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULTS;
    const parsed = JSON.parse(raw) as Partial<Dice3DDeviceSettings>;
    return {
      color: typeof parsed.color === "string" ? parsed.color : DEFAULTS.color,
      labelColor: typeof parsed.labelColor === "string" ? parsed.labelColor : DEFAULTS.labelColor,
      material:
        parsed.material === "metal" || parsed.material === "glass" || parsed.material === "plastic"
          ? parsed.material
          : DEFAULTS.material,
    };
  } catch {
    return DEFAULTS;
  }
}

/**
 * Persists the per-device 3D-dice appearance override to `localStorage`.
 * @param settings The settings to persist.
 * @example
 * ```ts
 * import { writeDice3DSettings } from "@shadowcat/module-dice-3d";
 *
 * writeDice3DSettings({ color: "#2d6ee8", labelColor: "", material: "metal" });
 * ```
 */
export function writeDice3DSettings(settings: Dice3DDeviceSettings): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}
