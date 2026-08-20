// Framework-neutral i18n primitive: subscribe/snapshot like DocumentStore and
// ContributionRegistry, so any framework (Svelte via createSubscriber, Vue, …)
// can read t() reactively. Minimal {name} interpolation; ICU/plural deferred.
/** One locale's flat key→message catalog. */
export type Messages = Record<string, string>;
/** Interpolation values for a `t()` call's `{name}` placeholders. */
export type I18nParams = Record<string, string | number>;
/** A `subscribe()` callback, invoked with no arguments on locale change. */
export type Listener = () => void;

/** One `(locale, key)` pair a module's `addMessages` call inserted; recorded so
 * `I18n.removeModule` can undo precisely those insertions on unload. Not exported. */
interface ModuleMessageKey {
  /** The locale the key was inserted into. */
  locale: string;
  /** The inserted key. */
  key: string;
}

/** Options for `I18n.addMessages`. */
export interface AddMessagesOptions {
  /** The registering module's id, recorded for `removeModule` teardown; omitted for a
   * host-registered (non-module) call. */
  module?: string;
}

/** The framework-neutral i18n primitive: current locale, per-locale message
 * catalogs, and `subscribe`/snapshot reactivity (mirrors `DocumentStore` and
 * `ContributionRegistry`) so any framework (Svelte via `createSubscriber`, Vue,
 * …) can read `t()` reactively. Minimal `{name}` interpolation only — ICU/plural
 * support is deferred. */
export class I18n {
  /** The active locale key. */
  #locale: string;
  /** Every loaded catalog, keyed by locale. */
  #catalogs: Record<string, Messages>;
  /** Subscribers notified on a locale change that actually took effect. */
  #listeners = new Set<Listener>();
  /** Tracks which `(locale, key)` pairs a module's `addMessages` call inserted, so
   * `removeModule` can undo precisely those insertions on unload. Not stamped for
   * a host-registered (non-module) call. */
  #moduleKeys = new Map<string, Set<ModuleMessageKey>>();

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

  /** Merges `messages` into `locale`'s catalog — new keys are added, an existing key (including a
   * BUILT-IN one) is overwritten (last write wins; no collision arbitration beyond that). Notifies
   * subscribers only if `locale` is the currently active one (a change to a non-active locale's
   * catalog has nothing observable to react to yet).
   * @param locale The locale to merge into; created if it doesn't already have a catalog.
   * @param messages The key→message fragment to merge in.
   * @param opts Registration options.
   * @param opts.module The registering module's id, recorded for `removeModule` teardown;
   * omitted for a host-registered (non-module) call.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: {} });
   * i18n.addMessages("en", { "myModule.greeting": "Hi!" }, { module: "my-module" });
   * i18n.t("myModule.greeting"); // "Hi!"
   * ```
   */
  addMessages(locale: string, messages: Messages, opts: AddMessagesOptions = {}): void {
    const catalog = this.#catalogs[locale] ?? (this.#catalogs[locale] = {});
    for (const [key, value] of Object.entries(messages)) {
      catalog[key] = value;
      if (opts.module) {
        const set = this.#moduleKeys.get(opts.module) ?? new Set();
        set.add({ locale, key });
        this.#moduleKeys.set(opts.module, set);
      }
    }
    if (locale === this.#locale) {
      for (const fn of this.#listeners) fn();
    }
  }

  /** Removes every `(locale, key)` pair `moduleId` inserted via `addMessages` (module unload
   * teardown) — a no-op if the module never called `addMessages`. Does NOT restore a value the
   * module's insertion overwrote (e.g. a shadowed built-in key); the key is simply absent
   * afterward, falling back to the raw key string in `t()` like any other missing key.
   * @param moduleId The module id whose inserted messages should be removed.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: {} });
   * i18n.addMessages("en", { "myModule.greeting": "Hi!" }, { module: "my-module" });
   * i18n.removeModule("my-module");
   * i18n.t("myModule.greeting"); // "myModule.greeting" (fell back to the key)
   * ```
   */
  removeModule(moduleId: string): void {
    const keys = this.#moduleKeys.get(moduleId);
    if (!keys) return;
    let changed = false;
    for (const { locale, key } of keys) {
      if (this.#catalogs[locale] && key in this.#catalogs[locale]) {
        delete this.#catalogs[locale][key];
        changed = true;
      }
    }
    this.#moduleKeys.delete(moduleId);
    if (changed && this.#catalogs[this.#locale]) {
      for (const fn of this.#listeners) fn();
    }
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
