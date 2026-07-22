// Framework-neutral live counter backing the chat panel's `PanelMeta.badge`
// (the panel-manager tab renderer's subscribe/get seam — see `PanelBadge` in
// @shadowcat/core's contributions.ts). A `PanelMeta` object is registered
// once at module install; this instance is that registration's `badge`
// field, so its count can change on every message arrival without ever
// touching the panel-manager's own layout/meta reconcile.
import type { PanelBadge } from "@shadowcat/core";

export class ChatUnreadBadge implements PanelBadge {
  #count = 0;
  #listeners = new Set<() => void>();

  get(): number {
    return this.#count;
  }

  set(count: number): void {
    if (count === this.#count) return;
    this.#count = count;
    for (const cb of this.#listeners) cb();
  }

  subscribe(cb: () => void): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }
}

/** Module-singleton: `index.ts` contributes it as the chat panel's
 * `PanelMeta.badge`; `ChatPanel.svelte` (same module) is the only writer. */
export const chatUnreadBadge = new ChatUnreadBadge();
