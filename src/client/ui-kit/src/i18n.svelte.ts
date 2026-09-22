import { createSubscriber } from "svelte/reactivity";
import { I18n, type I18nParams } from "@shadowcat/core";
import { en } from "./locales/en";

/** The app's single i18n instance (one `en` catalog for now). */
export const i18n = new I18n("en", { en });

const subscribe = createSubscriber((update) => i18n.subscribe(update));

/** Reactive translate: reading it in a rune context (`$derived`, `$effect`, a
 * component's template) re-runs on `setLocale` from any source, via the shared
 * `subscribe` (a `createSubscriber` wrapping `i18n.subscribe`). A key absent from the ACTIVE
 * locale's catalog is looked up in `I18n`'s `fallbackLocale` (`"en"` for this instance) before
 * falling through to the raw key itself.
 * @param key - The message key to resolve.
 * @param params - Interpolation values for `{name}`-style placeholders; omitted
 * skips interpolation entirely.
 * @returns The resolved (and interpolated, if `params` given) message, or `key`
 * verbatim if neither the active nor the fallback locale has an entry for it.
 * @example t("settings.role", { role: "gm" });
 */
export function t(key: string, params?: I18nParams): string {
  subscribe();
  return i18n.t(key, params);
}

/** Reactive plural selection over the shared `i18n` instance — see `I18n.plural`. Re-runs on
 * `setLocale` from any source, like `t`.
 * @param key - The catalog key naming a `PluralEntry`.
 * @param count - The quantity selecting the plural category.
 * @param params - Additional interpolation values; `{count}` is provided automatically.
 * @returns The resolved plural message for `count`'s category, or the raw key if no locale in
 * the chain has a `PluralEntry` at `key`.
 * @example plural("chat.memberCount", 3);
 */
export function plural(key: string, count: number, params?: I18nParams): string {
  subscribe();
  return i18n.plural(key, count, params);
}

/** Reactive locale-aware string comparator over the shared `i18n` instance — see
 * `I18n.compare`. The ONE collator every locale-sensitive sort in the client reads; a call site
 * must never construct its own `Intl.Collator`. Re-runs on `setLocale` from any source, like `t`.
 * @param a - The first string.
 * @param b - The second string.
 * @returns Negative, zero, or positive per `Array.prototype.sort`'s contract.
 * @example ["b", "a"].sort(compare); // ["a", "b"]
 */
export function compare(a: string, b: string): number {
  subscribe();
  return i18n.compare(a, b);
}

/** The current locale, read reactively — invalidates on setLocale from any
 * source (the Settings switcher, session-restore, etc.).
 * @returns The active locale key (e.g. `"en"`).
 * @example locale(); // "en"
 */
export function locale(): string {
  subscribe();
  return i18n.locale;
}

/** The reactive translate function's shape, matching `AppContext.t`. */
export type TFunc = typeof t;
