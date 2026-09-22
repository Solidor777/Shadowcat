// Framework-neutral i18n primitive: subscribe/snapshot like DocumentStore and
// ContributionRegistry, so any framework (Svelte via createSubscriber, Vue, …)
// can read t() reactively. Minimal {name} interpolation; plural selection rides
// Intl.PluralRules (see `plural`) rather than the full ICU MessageFormat grammar.
/** One locale's flat key→message catalog. A catalog entry is either a plain string (looked up
 * via `t()`) or a plural-category map (looked up via `plural()`). */
export type Messages = Record<string, string | PluralEntry>;
/** A catalog entry supporting `plural()`: one message per CLDR plural category this catalog
 * chooses to distinguish. `other` is REQUIRED — every locale's plural rule set includes it, and
 * `Intl.PluralRules.select` never returns a category the entry doesn't have unless the catalog
 * omits it, in which case `plural()` falls through to `other`. */
export interface PluralEntry {
  /** The `"other"` category message — every locale defines it; the universal fallback. */
  other: string;
  /** The `"zero"` category message, for locales that distinguish it (e.g. Arabic). */
  zero?: string;
  /** The `"one"` category message (e.g. English/French singular). */
  one?: string;
  /** The `"two"` category message, for locales that distinguish it (e.g. Arabic, Welsh). */
  two?: string;
  /** The `"few"` category message, for locales that distinguish it (e.g. Polish, Russian). */
  few?: string;
  /** The `"many"` category message, for locales that distinguish it (e.g. Polish, Russian). */
  many?: string;
}
/** Interpolation values for a `t()`/`plural()` call's `{name}` placeholders. */
export type I18nParams = Record<string, string | number>;
/** A `subscribe()` callback, invoked with no arguments on locale change. */
export type Listener = () => void;
/** The built-in fallback locale consulted when the active locale's catalog lacks a key/entry,
 * before falling back further to the raw key string. */
const DEFAULT_FALLBACK_LOCALE = "en";

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
 * …) can read `t()` reactively. Minimal `{name}` interpolation; plural selection
 * (`plural()`) rides `Intl.PluralRules` rather than the full ICU MessageFormat
 * grammar. A key/entry missing from the active locale's catalog is looked up in
 * `fallbackLocale` before falling through to the raw key string. */
export class I18n {
  /** The active locale key. */
  #locale: string;
  /** Every loaded catalog, keyed by locale. */
  #catalogs: Record<string, Messages>;
  /** The fallback locale consulted before a lookup falls through to the raw key. */
  #fallbackLocale: string;
  /** Subscribers notified on a locale change that actually took effect. */
  #listeners = new Set<Listener>();
  /** Tracks which `(locale, key)` pairs a module's `addMessages` call inserted, so
   * `removeModule` can undo precisely those insertions on unload. Not stamped for
   * a host-registered (non-module) call. */
  #moduleKeys = new Map<string, Set<ModuleMessageKey>>();
  /** `Intl.Collator` bound to the active locale, rebuilt on `setLocale`; `compare()`'s single
   * source, so no adopted call site constructs its own collator. */
  #collator: Intl.Collator;
  /** `Intl.PluralRules` cache, one per locale, built lazily. Category selection must use the
   * rules of the locale that SUPPLIED the matched `PluralEntry` — never the active locale's own
   * rules — or a fallback-locale entry selects its category under the wrong grammar (e.g. with
   * `fr` active and only an `en` entry, `Intl.PluralRules('fr').select(0)` is `"one"` while
   * `Intl.PluralRules('en').select(0)` is `"other"`, rendering "0 item" instead of "0 items"). */
  #pluralRulesByLocale = new Map<string, Intl.PluralRules>();

  /** Builds an `I18n` instance already loaded with every catalog.
   * @param locale The active locale key (e.g. `"en"`); must be a key of `catalogs`.
   * @param catalogs Every available locale's message catalog, keyed by locale.
   * @param fallbackLocale The locale consulted before a missing key/entry falls through to the
   * raw key string; defaults to `"en"`. Never itself falls further (a `fallbackLocale` catalog
   * miss goes straight to the raw key).
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: { "hello": "Hello, {name}!" } });
   * ```
   */
  constructor(locale: string, catalogs: Record<string, Messages>, fallbackLocale = DEFAULT_FALLBACK_LOCALE) {
    this.#locale = locale;
    this.#catalogs = catalogs;
    this.#fallbackLocale = fallbackLocale;
    this.#collator = new Intl.Collator(locale);
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
    this.#collator = new Intl.Collator(locale);
    for (const fn of this.#listeners) fn();
  }

  /** Returns the cached `Intl.PluralRules` for `locale`, building and caching one on first use.
   * @param locale The locale to resolve plural-category selection rules for.
   * @returns The (cached) `Intl.PluralRules` bound to `locale`.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from plural()
   * declare const self: I18n;
   * ```
   */
  #pluralRulesFor(locale: string): Intl.PluralRules {
    let rules = this.#pluralRulesByLocale.get(locale);
    if (!rules) {
      rules = new Intl.PluralRules(locale);
      this.#pluralRulesByLocale.set(locale, rules);
    }
    return rules;
  }

  /** Look up `key` in the active locale, then `fallbackLocale`; missing from both → the key
   * itself. A `PluralEntry` found at `key` (a `plural()` catalog entry looked up through `t()`)
   * is treated as a miss for THIS locale in the chain, since `t()` has no count to select a
   * category with — the chain still falls through to the next locale/the raw key rather than
   * stringifying the entry.
   * @param key The message key to resolve.
   * @param params Interpolation values for `{name}` placeholders in the
   * resolved message; omitted skips interpolation entirely.
   * @returns The resolved (and interpolated, if `params` given) message, or
   * `key` verbatim if neither the active nor the fallback locale has an entry for it.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: { hello: "Hello, {name}!" } });
   * i18n.t("hello", { name: "world" }); // "Hello, world!"
   * ```
   */
  t(key: string, params?: I18nParams): string {
    const msg = this.#stringLookup(this.#locale, key) ?? this.#stringLookup(this.#fallbackLocale, key) ?? key;
    return params ? interpolate(msg, params) : msg;
  }

  /** Look up `key` in `locale`'s catalog as a plain string entry; a `PluralEntry` or a missing
   * key/catalog both read as absent.
   * @param locale The locale to look up in.
   * @param key The message key.
   * @returns The string entry, or `undefined` if absent or non-string.
   * @example
   * ```
   * // private helper; consumed by t()
   * declare const self: I18n;
   * ```
   */
  #stringLookup(locale: string, key: string): string | undefined {
    const entry = this.#catalogs[locale]?.[key];
    return typeof entry === "string" ? entry : undefined;
  }

  /** Selects a plural-category message for `count`, via `Intl.PluralRules` bound to the active
   * locale, falling through `fallbackLocale` then the raw key exactly like `t()`. A `count`
   * placeholder is available for interpolation automatically (merged under `params`, `params`
   * taking precedence on a collision) so a caller rarely needs to pass it explicitly.
   * @param key The catalog key naming a `PluralEntry`.
   * @param count The quantity selecting the plural category.
   * @param params Additional interpolation values; `{count}` is provided automatically.
   * @returns The resolved (and interpolated) message for `count`'s plural category, `other` if
   * the entry lacks that category, or the raw key if no locale in the chain has a `PluralEntry`
   * at `key`.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: { items: { one: "{count} item", other: "{count} items" } } });
   * i18n.plural("items", 1); // "1 item"
   * i18n.plural("items", 5); // "5 items"
   * ```
   */
  plural(key: string, count: number, params?: I18nParams): string {
    const activeEntry = this.#pluralLookup(this.#locale, key);
    const [entry, sourceLocale] = activeEntry
      ? [activeEntry, this.#locale]
      : [this.#pluralLookup(this.#fallbackLocale, key), this.#fallbackLocale];
    if (!entry) return key;
    const category = this.#pluralRulesFor(sourceLocale).select(count);
    const msg = entry[category] ?? entry.other;
    return interpolate(msg, { count, ...params });
  }

  /** Look up `key` in `locale`'s catalog as a `PluralEntry`; a plain string or a missing
   * key/catalog both read as absent.
   * @param locale The locale to look up in.
   * @param key The message key.
   * @returns The `PluralEntry`, or `undefined` if absent or a plain string.
   * @example
   * ```
   * // private helper; consumed by plural()
   * declare const self: I18n;
   * ```
   */
  #pluralLookup(locale: string, key: string): PluralEntry | undefined {
    const entry = this.#catalogs[locale]?.[key];
    return typeof entry === "object" ? entry : undefined;
  }

  /** Compares two strings under the active locale's collation order (`Intl.Collator`, rebuilt
   * on `setLocale`) — the one collator every locale-aware sort in the client reads, rather than
   * each call site constructing its own.
   * @param a The first string.
   * @param b The second string.
   * @returns Negative, zero, or positive per `Array.prototype.sort`'s contract.
   * @example
   * ```ts
   * import { I18n } from "@shadowcat/core";
   *
   * const i18n = new I18n("en", { en: {} });
   * ["b", "a"].sort((x, y) => i18n.compare(x, y)); // ["a", "b"]
   * ```
   */
  compare(a: string, b: string): number {
    return this.#collator.compare(a, b);
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
