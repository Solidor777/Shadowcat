/** The storage surface `ShareRulerPreference` persists through — the `getItem`/`setItem` subset
 * of the DOM `Storage`, so the shell passes `localStorage` and a test passes a plain object. */
export type ShareRulerStorage = Pick<Storage, "getItem" | "setItem">;

/** The storage key holding `user`'s persisted preference: `"1"` when sharing is on, `"0"` when
 * off. Keyed per user because `localStorage` is shared by every account on a browser profile,
 * and one account's opt-in to broadcasting its rulers must not switch it on for another.
 * @param user The user id the preference belongs to.
 * @returns The storage key for that user's preference.
 * @example
 * ```ts
 * import { shareRulerStorageKey } from "@shadowcat/ui-kit";
 *
 * shareRulerStorageKey("u1") !== shareRulerStorageKey("u2"); // true
 * ```
 */
export function shareRulerStorageKey(user: string): string {
  return `shadowcat.shareRuler:${user}`;
}

/**
 * Whether THIS user broadcasts their own measure-tool drags to the other members — the shared
 * live ruler's toggle. A per-user, per-device preference, never a document field and never sent
 * to the server: only the ruler's points travel, and only while this is on. It defaults to OFF,
 * so with sharing off no route is sent (the one frame it can still cause is the terminating stop of
 * a ruler already shown). Persisted through an injected storage (`localStorage` in the shell) under a key
 * naming the user (`shareRulerStorageKey`) so a chosen value survives reload and does not leak to
 * another account on the same browser; an absent, unreadable or unrecognized stored value reads
 * as off.
 *
 * A stable instance is held by the shell and shared via AppContext: `ToolRail`'s toggle writes
 * it and the measure tool reads it on every drag update. Sibling of `SnapModePreference`: the
 * same stable-instance/mutate-in-place `$state` shape, orthogonal to the document stream.
 */
export class ShareRulerPreference {
  /** Backing store for {@link ShareRulerPreference.enabled}. */
  #enabled = $state(false);
  /** Where `set` persists to; `undefined` keeps the value in memory only. */
  #storage: ShareRulerStorage | undefined;
  /** The storage key of this user's preference. */
  #key: string;

  /** Creates the preference for `user`, restoring a persisted value when `storage` holds one.
   * @param user - The id of the user the preference belongs to.
   * @param storage - Where to persist (the shell passes `localStorage`); omit for in-memory.
   * @example const pref = new ShareRulerPreference("user-1", localStorage);
   */
  constructor(user: string, storage?: ShareRulerStorage) {
    this.#storage = storage;
    this.#key = shareRulerStorageKey(user);
    try {
      this.#enabled = storage?.getItem(this.#key) === "1";
    } catch {
      this.#enabled = false;
    }
  }

  /** Whether this user's measure-tool drags are broadcast. `false` (the default) sends no route;
   * the one frame it can still cause is the terminating stop of a ruler already shown.
   * @returns `true` when sharing is on. */
  get enabled(): boolean {
    return this.#enabled;
  }

  /** Turn sharing on or off and persist the choice. A storage that throws (quota, privacy
   * mode) leaves the in-memory value applied for the session.
   * @param enabled - `true` to broadcast this user's rulers.
   * @example shareRuler.set(true);
   */
  set(enabled: boolean): void {
    this.#enabled = enabled;
    try {
      this.#storage?.setItem(this.#key, enabled ? "1" : "0");
    } catch {
      // Persistence is best-effort: the in-memory value already governs this session.
    }
  }
}
