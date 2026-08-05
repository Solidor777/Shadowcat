// Framework-neutral live counter backing the chat panel's `PanelMeta.badge`
// (the panel-manager tab renderer's subscribe/get seam — see `PanelBadge` in
// @shadowcat/core's contributions.ts). A `PanelMeta` object is registered
// once at module install; this instance is that registration's `badge`
// field, so its count can change on every message arrival without ever
// touching the panel-manager's own layout/meta reconcile.
import type { PanelBadge } from "@shadowcat/core";

/**
 * A `PanelBadge` implementation: a live count with get/set/subscribe, so a
 * `PanelTabRenderer` (src/modules/panels/src/engine/dockview.ts:308) can
 * re-render a tab's badge on every count change without the panel-manager's
 * own layout/meta reconcile ever running.
 * @example
 * ```
 * // private module; not reachable as a workspace import — call shape:
 * const badge = new ChatUnreadBadge();
 * badge.subscribe(() => {});
 * badge.set(3);
 * ```
 */
export class ChatUnreadBadge implements PanelBadge {
  #count = 0;
  #listeners = new Set<() => void>();

  /**
   * The current count.
   * @returns The current count.
   * @example
   * ```
   * // private method; not reachable as a workspace import — call shape:
   * badge.get();
   * ```
   */
  get(): number {
    return this.#count;
  }

  /**
   * Sets the count and notifies every subscriber — but only when the count
   * actually changed; an unchanged `set` is a silent no-op, so a subscriber
   * that re-renders on notification never re-renders for a no-op update.
   * @param count The new count.
   * @example
   * ```
   * // private method; not reachable as a workspace import — call shape:
   * badge.set(3);
   * ```
   */
  set(count: number): void {
    if (count === this.#count) return;
    this.#count = count;
    for (const cb of this.#listeners) cb();
  }

  /**
   * Registers `cb` to be called (with no arguments) on every count change.
   * @param cb Called after each `set` that actually changes the count.
   * @returns A disposer that removes `cb`; safe to call more than once.
   * @example
   * ```
   * // private method; not reachable as a workspace import — call shape:
   * const unsubscribe = badge.subscribe(() => {});
   * ```
   */
  subscribe(cb: () => void): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }
}

/**
 * Module-singleton: `index.ts` contributes it as the chat panel's
 * `PanelMeta.badge`; `ChatPanel.svelte` (same module, :63) is the only
 * production writer (verified: `.set(` has no other call site in this
 * package outside test files). Its subscribers are whichever
 * `PanelTabRenderer`s are currently mounted for this tab
 * (src/modules/panels/src/engine/dockview.ts:308); each disposes its own
 * subscription in its own `dispose()` (dockview.ts:405-408), so a tab
 * close/reopen does not accumulate listeners here as long as dockview-core
 * calls that lifecycle hook on every teardown — a third-party guarantee this
 * file does not itself enforce or verify.
 */
export const chatUnreadBadge = new ChatUnreadBadge();
