// Framework-neutral i18n primitive: subscribe/snapshot like DocumentStore and
// ContributionRegistry, so any framework (Svelte via createSubscriber, Vue, …)
// can read t() reactively. Minimal {name} interpolation; ICU/plural deferred.
export type Messages = Record<string, string>;
export type I18nParams = Record<string, string | number>;
export type Listener = () => void;

/** The framework-neutral i18n primitive: current locale, per-locale message
 * catalogs, and `subscribe`/snapshot reactivity (mirrors `DocumentStore` and
 * `ContributionRegistry`) so any framework (Svelte via `createSubscriber`, Vue,
 * …) can read `t()` reactively. Minimal `{name}` interpolation only — ICU/plural
 * support is deferred. */
export class I18n {
  #locale: string;
  #catalogs: Record<string, Messages>;
  #listeners = new Set<Listener>();

  /** Builds an `I18n` instance already loaded with every catalog.
   * @param locale The active locale key (e.g. `"en"`); must be a key of `catalogs`.
   * @param catalogs Every available locale's message catalog, keyed by locale.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: { "hello": "Hello, {name}!" } });
   * ```
   */
  constructor(locale: string, catalogs: Record<string, Messages>) {
    this.#locale = locale;
    this.#catalogs = catalogs;
  }

  /** The active locale key.
   * @returns The active locale key.
   */
  get locale(): string {
    return this.#locale;
  }

  /** Every locale key with a loaded catalog.
   * @returns Every locale key with a loaded catalog.
   */
  get locales(): string[] {
    return Object.keys(this.#catalogs);
  }

  /** Switches the active locale and notifies subscribers. A no-op (no
   * notification) if `locale` is already active.
   * @param locale The locale key to switch to; does not need to have a loaded
   * catalog — `t()` falls back to the raw key for any lookup that misses.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: {}, fr: {} });
   * i18n.setLocale("fr");
   * ```
   */
  setLocale(locale: string): void {
    if (locale === this.#locale) return;
    this.#locale = locale;
    for (const fn of this.#listeners) fn();
  }

  /** Look up `key` in the current locale; missing key → the key itself.
   * @param key The message key to resolve.
   * @param params Interpolation values for `{name}` placeholders in the
   * resolved message; omitted skips interpolation entirely.
   * @returns The resolved (and interpolated, if `params` given) message, or
   * `key` verbatim if the current locale has no entry for it.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: { hello: "Hello, {name}!" } });
   * i18n.t("hello", { name: "world" }); // "Hello, world!"
   * ```
   */
  t(key: string, params?: I18nParams): string {
    const msg = this.#catalogs[this.#locale]?.[key] ?? key;
    return params ? interpolate(msg, params) : msg;
  }

  /** Notifies `listener` on every `setLocale` call that actually changes the locale.
   * @param listener Called with no arguments after a locale change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: {} });
   * const unsubscribe = i18n.subscribe(() => {});
   * unsubscribe();
   * ```
   */
  subscribe(listener: Listener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }
}

/** Replace `{name}` with params; an unknown placeholder is left intact.
 * @param msg The message template containing `{name}`-style placeholders.
 * @param params Interpolation values, keyed by placeholder name.
 * @returns `msg` with every placeholder present in `params` substituted.
 * @example
 * ```
 * // internal helper; not part of the public API
 * interpolate("Hello, {name}!", { name: "world" }); // "Hello, world!"
 * ```
 */
function interpolate(msg: string, params: I18nParams): string {
  return msg.replace(/\{(\w+)\}/g, (_, k: string) =>
    k in params ? String(params[k]) : `{${k}}`,
  );
}
