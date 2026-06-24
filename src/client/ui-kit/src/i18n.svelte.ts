import { createSubscriber } from "svelte/reactivity";
import { I18n, type I18nParams } from "@shadowcat/core";
import { en } from "./locales/en";

/** The app's single i18n instance (one `en` catalog for now). */
export const i18n = new I18n("en", { en });

const subscribe = createSubscriber((update) => i18n.subscribe(update));

/** Reactive translate: reading it in a rune context re-runs on setLocale. */
export function t(key: string, params?: I18nParams): string {
  subscribe();
  return i18n.t(key, params);
}

/** The current locale, read reactively — invalidates on setLocale from any
 * source (the Settings switcher, M7d-3 session-restore, etc.). */
export function locale(): string {
  subscribe();
  return i18n.locale;
}

export type TFunc = typeof t;
