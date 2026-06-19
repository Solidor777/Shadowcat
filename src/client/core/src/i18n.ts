// Framework-neutral i18n primitive: subscribe/snapshot like DocumentStore and
// ContributionRegistry, so any framework (Svelte via createSubscriber, Vue, …)
// can read t() reactively. Minimal {name} interpolation; ICU/plural deferred.
export type Messages = Record<string, string>;
export type I18nParams = Record<string, string | number>;
export type Listener = () => void;

export class I18n {
  #locale: string;
  #catalogs: Record<string, Messages>;
  #listeners = new Set<Listener>();

  constructor(locale: string, catalogs: Record<string, Messages>) {
    this.#locale = locale;
    this.#catalogs = catalogs;
  }

  get locale(): string {
    return this.#locale;
  }

  get locales(): string[] {
    return Object.keys(this.#catalogs);
  }

  setLocale(locale: string): void {
    if (locale === this.#locale) return;
    this.#locale = locale;
    for (const fn of this.#listeners) fn();
  }

  /** Look up `key` in the current locale; missing key → the key itself. */
  t(key: string, params?: I18nParams): string {
    const msg = this.#catalogs[this.#locale]?.[key] ?? key;
    return params ? interpolate(msg, params) : msg;
  }

  subscribe(listener: Listener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }
}

/** Replace `{name}` with params; an unknown placeholder is left intact. */
function interpolate(msg: string, params: I18nParams): string {
  return msg.replace(/\{(\w+)\}/g, (_, k: string) =>
    k in params ? String(params[k]) : `{${k}}`,
  );
}
