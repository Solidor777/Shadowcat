// The ThemeController singleton and its Svelte adapter, mirroring the i18n
// seam's shape: a `$state`-backed controller with subscribe/snapshot
// reactivity, plus a `createSubscriber`-backed reactive read for components.
// Application writes every token and the color scheme as inline styles on each
// themed document's root element — the main document plus every registered
// secondary document — so inline styles beat both the shell stylesheet and any
// cloned stylesheets, and every open window follows a theme swap.
import { createSubscriber } from "svelte/reactivity";
import {
  BUILTIN_THEMES,
  DEFAULT_THEME_ID,
  resolveTheme,
  sanitizeCustomThemes,
  type CustomTheme,
  type ThemeDefinition,
} from "./theme";

/** The persisted theme preference: the active selector plus the saved custom
 * themes. This is the shape `ThemeController.serialize` emits and
 * `ThemeController.load` accepts. */
export interface PersistedTheme {
  /** The active selector: a built-in theme id or `custom:<id>`. */
  active: string;
  /** Saved custom themes, keyed by id. */
  custom: Record<string, CustomTheme>;
}

/** A no-argument callback invoked after any theme state change. */
export type ThemeListener = () => void;

/** Owns the active theme and the saved custom themes, applies the resolved
 * theme to every registered document, and notifies subscribers on change.
 * Framework-neutral consumers use `subscribe`; Svelte consumers read through
 * the module-level reactive adapter. */
export class ThemeController {
  /** Backing store for {@link ThemeController.active}. */
  #active = $state(DEFAULT_THEME_ID);
  /** Backing store for {@link ThemeController.customThemes}. */
  #custom = $state<Record<string, CustomTheme>>({});
  /** Secondary documents the resolved theme is applied to on every change. */
  #documents = new Set<Document>();
  /** Subscribers notified after a change that actually took effect. */
  #listeners = new Set<ThemeListener>();

  /** The active selector: a built-in theme id or `custom:<id>`. Always
   * resolvable — an unresolvable value never enters the controller.
   * @returns The active selector. */
  get active(): string {
    return this.#active;
  }

  /** The resolved active theme — the built-in itself, or a custom theme's
   * validated overrides layered onto its built-in base.
   * @returns The resolved theme definition. */
  get resolved(): ThemeDefinition {
    return resolveTheme(this.#active, this.#custom);
  }

  /** A snapshot of the saved custom themes, keyed by id.
   * @returns The custom theme map (a copy — mutating it changes nothing). */
  get customThemes(): Readonly<Record<string, CustomTheme>> {
    return { ...this.#custom };
  }

  /** Switches the active theme and re-applies it to every registered
   * document. An unresolvable id (neither a built-in nor a saved
   * `custom:<id>`) selects the default theme instead. Selecting the already
   * active id is a no-op.
   * @param id The selector to activate: a built-in theme id or `custom:<id>`.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * theme.setActive("slate-light");
   * ```
   */
  setActive(id: string): void {
    const next = this.#resolvable(id) ? id : DEFAULT_THEME_ID;
    if (next === this.#active) return;
    this.#active = next;
    this.#changed();
  }

  /** Saves a custom theme under `id` (creating or overwriting) and re-applies.
   * Token overrides are validated — unknown token keys are dropped. Does not
   * change the active selector.
   * @param id The custom theme id (the part after `custom:`).
   * @param custom The theme to save.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * theme.saveCustom("mine", { label: "Mine", base: "slate-dark", tokens: { accent: "#123456" } });
   * ```
   */
  saveCustom(id: string, custom: CustomTheme): void {
    const sanitized = sanitizeCustomThemes({ [id]: custom });
    this.#custom = { ...this.#custom, [id]: sanitized[id]! };
    this.#changed();
  }

  /** Deletes a saved custom theme and re-applies. If the deleted theme was
   * active, the default theme becomes active. Deleting an unknown id is a
   * no-op.
   * @param id The custom theme id to delete.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * theme.deleteCustom("mine");
   * ```
   */
  deleteCustom(id: string): void {
    if (!(id in this.#custom)) return;
    const rest = { ...this.#custom };
    delete rest[id];
    this.#custom = rest;
    if (this.#active === `custom:${id}`) this.#active = DEFAULT_THEME_ID;
    this.#changed();
  }

  /** Replaces the whole state from a persisted blob, tolerating garbage:
   * `undefined`, an unresolvable active selector, and malformed custom entries
   * all fall back to the default theme and an empty custom map.
   * @param state The persisted state, or `undefined` when none was saved.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * theme.load({ active: "slate-light", custom: {} });
   * ```
   */
  load(state: PersistedTheme | undefined): void {
    const custom = sanitizeCustomThemes(state?.custom);
    const requested = typeof state?.active === "string" ? state.active : DEFAULT_THEME_ID;
    const active =
      BUILTIN_THEMES.some((theme) => theme.id === requested) ||
      (requested.startsWith("custom:") && requested.slice("custom:".length) in custom)
        ? requested
        : DEFAULT_THEME_ID;
    this.#custom = custom;
    this.#active = active;
    this.#changed();
  }

  /** The persisted shape: active selector plus custom themes.
   * @returns A snapshot suitable for persistence.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * const state = theme.serialize();
   * ```
   */
  serialize(): PersistedTheme {
    const custom: Record<string, CustomTheme> = {};
    for (const [id, entry] of Object.entries(this.#custom)) {
      custom[id] = { ...entry, tokens: { ...entry.tokens } };
    }
    return { active: this.#active, custom };
  }

  /** Notifies `listener` after every state change that took effect.
   * @param listener Called with no arguments after a change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * const unsubscribe = theme.subscribe(() => {});
   * unsubscribe();
   * ```
   */
  subscribe(listener: ThemeListener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Writes every token and the color scheme of the resolved theme as inline
   * styles on `doc`'s root element. Values equal to the stylesheet defaults
   * are still written — application stays single-pathed, with no
   * removeProperty asymmetry.
   * @param doc The document to theme.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * theme.applyTo(document);
   * ```
   */
  applyTo(doc: Document): void {
    const resolved = this.resolved;
    const style = doc.documentElement.style;
    for (const [name, value] of Object.entries(resolved.tokens)) {
      style.setProperty(`--${name}`, value);
    }
    style.setProperty("color-scheme", resolved.colorScheme);
  }

  /** Registers an additional document (a secondary window) so it receives the
   * current theme immediately and every later change, until the returned
   * unregister function runs.
   * @param doc The document to track.
   * @returns The unregister function.
   * @example
   * ```ts
   * import { theme } from "@shadowcat/ui-kit";
   *
   * const unregister = theme.registerDocument(document);
   * unregister();
   * ```
   */
  registerDocument(doc: Document): () => void {
    this.#documents.add(doc);
    this.applyTo(doc);
    return () => this.#documents.delete(doc);
  }

  /** Whether `id` names a built-in or a saved custom theme.
   * @param id The selector to check.
   * @returns True when `id` resolves to a theme.
   * @example
   * ```ts
   * // internal helper; not part of the public API
   * this.#resolvable("slate-dark"); // true
   * ```
   */
  #resolvable(id: string): boolean {
    return (
      BUILTIN_THEMES.some((theme) => theme.id === id) ||
      (id.startsWith("custom:") && id.slice("custom:".length) in this.#custom)
    );
  }

  /** Re-applies the theme everywhere and notifies subscribers. The main
   * document access is guarded so construction and mutation survive jsdom/SSR
   * contexts without a DOM.
   * @example
   * ```ts
   * // internal helper; not part of the public API
   * this.#changed();
   * ```
   */
  #changed(): void {
    if (typeof document !== "undefined") this.applyTo(document);
    for (const doc of this.#documents) this.applyTo(doc);
    for (const fn of this.#listeners) fn();
  }
}

/** The app's single theme controller instance (exported like the i18n
 * singleton). */
export const theme = new ThemeController();

const subscribe = createSubscriber((update) => theme.subscribe(update));

/** The resolved active theme, read reactively: reading it in a rune context
 * (`$derived`, `$effect`, a component's template) re-runs on any theme change,
 * via the shared `subscribe` (a `createSubscriber` wrapping
 * `ThemeController.subscribe`).
 * @returns The resolved active theme definition.
 * @example activeTheme().colorScheme; // "dark"
 */
export function activeTheme(): ThemeDefinition {
  subscribe();
  return theme.resolved;
}
