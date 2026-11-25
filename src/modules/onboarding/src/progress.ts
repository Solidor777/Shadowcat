import type { TourMeta } from "@shadowcat/core";

/** The account-scoped settings seam the progress store writes through: a module's own
 * `ModuleContext.settings` `getUser`/`setUser` pair, which stores the slice under
 * `ui_state.modules.onboarding` through the merge-patch route. */
export interface ProgressSettings {
  /** Reads the module's account-scoped slice.
   * @returns The slice, or `{}` when unset. */
  getUser(): Record<string, unknown>;
  /** Merges `patch` into the slice; a `null` value removes that key.
   * @param patch The keys to merge.
   * @returns Resolves when the server accepted the patch. */
  setUser(patch: Record<string, unknown>): Promise<void>;
}

/** Slice key holding the account-wide "never offer tours" switch. */
export const DISABLED_KEY = "disabled";

/** The slice key holding the done-version of `tour`. One flat key per tour (never a nested
 * object) so a write replaces exactly one leaf: two sessions of one account finishing different
 * tours do not overwrite each other. A world-scoped tour is keyed per world, because one person
 * can be GM in one world and a player in another.
 * @param tour The tour whose done-state key to build.
 * @param world The current world id; used only for a `"world"`-scoped tour.
 * @returns The slice key.
 * @example
 * ```ts
 * import { doneKey } from "./progress";
 *
 * doneKey({ id: "core-ui.start", scope: "global" }, "w1"); // "g:core-ui.start"
 * doneKey({ id: "core-ui.start", scope: "world" }, "w1"); // "w:w1:core-ui.start"
 * ```
 */
export function doneKey(tour: Pick<TourMeta, "id" | "scope">, world: string): string {
  return tour.scope === "world" ? `w:${world}:${tour.id}` : `g:${tour.id}`;
}

/** Done-state for tours, persisted per account through `ProgressSettings`.
 *
 * Reads go through an in-memory overlay of this session's own writes layered over the settings
 * snapshot, so a tour finished a moment ago is never re-offered while its write is still in
 * flight. A tour is done when the stored version is at least the tour's `version`; bumping a
 * tour's version re-offers it. */
export class TourProgress {
  /** Writes made this session, applied over the settings snapshot on every read. */
  #local = new Map<string, unknown>();

  /** Builds the store over a module's settings seam.
   * @param settings The module's account-scoped settings.
   * @param onWriteError Called when a write to the server is rejected; the in-memory overlay
   * keeps the value for the rest of the session.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * ```
   */
  constructor(
    private readonly settings: ProgressSettings,
    private readonly onWriteError: (error: unknown) => void,
  ) {}

  /** Reads one slice key, preferring this session's own writes.
   * @param key The slice key.
   * @returns The value, or `undefined` when unset.
   * @example
   * ```ts
   * // private; used by `isDisabled` and `doneVersion`
   * ```
   */
  #read(key: string): unknown {
    return this.#local.has(key) ? this.#local.get(key) : this.settings.getUser()[key];
  }

  /** Writes one slice key through the settings seam, overlaying it locally first.
   * @param key The slice key.
   * @param value The value to store.
   * @example
   * ```ts
   * // private; used by `setDisabled` and `markDone`
   * ```
   */
  #write(key: string, value: unknown): void {
    this.#local.set(key, value);
    this.settings.setUser({ [key]: value }).catch(this.onWriteError);
  }

  /** Whether the user switched tours off for good ("Disable Tutorials").
   * @returns `true` when no tour should be offered automatically.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {}).isDisabled(); // false
   * ```
   */
  isDisabled(): boolean {
    return this.#read(DISABLED_KEY) === true;
  }

  /** Switches automatic offers off for this account.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {}).setDisabled();
   * ```
   */
  setDisabled(): void {
    this.#write(DISABLED_KEY, true);
  }

  /** The highest version of `tour` the user finished or skipped.
   * @param tour The tour to look up.
   * @param world The current world id, for a world-scoped tour.
   * @returns The stored version, or `0` when the tour was never finished.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * progress.doneVersion({ id: "a.b", scope: "global" }, "w1"); // 0
   * ```
   */
  doneVersion(tour: Pick<TourMeta, "id" | "scope">, world: string): number {
    const value = this.#read(doneKey(tour, world));
    return typeof value === "number" ? value : 0;
  }

  /** Records `tour`'s current version as finished or skipped.
   * @param tour The tour that ended.
   * @param world The current world id, for a world-scoped tour.
   * @example
   * ```ts
   * import { TourProgress } from "./progress";
   *
   * const progress = new TourProgress({ getUser: () => ({}), setUser: async () => {} }, () => {});
   * progress.markDone({ id: "a.b", scope: "global", version: 1 }, "w1");
   * ```
   */
  markDone(tour: Pick<TourMeta, "id" | "scope" | "version">, world: string): void {
    this.#write(doneKey(tour, world), tour.version);
  }
}
