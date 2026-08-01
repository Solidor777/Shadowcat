import { createSubscriber } from "svelte/reactivity";
import { I18n, type I18nParams } from "@shadowcat/core";
import { en } from "./locales/en";

/** The app's single i18n instance (one `en` catalog for now). */
export const i18n = new I18n("en", { en });

const subscribe = createSubscriber((update) => i18n.subscribe(update));

/** Reactive translate: reading it in a rune context (`$derived`, `$effect`, a
 * component's template) re-runs on `setLocale` from any source, via the shared
 * `subscribe` (a `createSubscriber` wrapping `i18n.subscribe`). There is no
 * cross-locale fallback: a key absent from the ACTIVE locale's catalog renders
 * as the raw key itself (core `I18n.t`'s behavior), even if another loaded
 * locale (e.g. `"en"`) has it.
 * @param key - The message key to resolve.
 * @param params - Interpolation values for `{name}`-style placeholders; omitted
 * skips interpolation entirely.
 * @returns The resolved (and interpolated, if `params` given) message, or `key`
 * verbatim if the active locale has no entry for it.
 * @example t("settings.role", { role: "gm" });
 */
export function t(key: string, params?: I18nParams): string {
  subscribe();
  return i18n.t(key, params);
}

/** The current locale, read reactively — invalidates on setLocale from any
 * source (the Settings switcher, M7d-3 session-restore, etc.).
 * @returns The active locale key (e.g. `"en"`).
 * @example locale(); // "en"
 */
export function locale(): string {
  subscribe();
  return i18n.locale;
}

export type TFunc = typeof t;
