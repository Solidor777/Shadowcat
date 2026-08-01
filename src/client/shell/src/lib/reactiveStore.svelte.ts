import { createSubscriber } from "svelte/reactivity";
import type { DocumentStore, WireDocument } from "@shadowcat/core";

/** Reactive `query` over a DocumentStore: reading it in a rune context re-runs
 * when the store emits. The same subscribe/snapshot bridge as <Surface>.
 * @param store - The document store to wrap.
 * @returns An object with `query(docType)`/`get(id)` accessors equivalent to
 *   the store's own, except each read re-runs its caller (`$derived`,
 *   `$effect`) whenever `store` emits an update.
 * @example
 * ```
 * const reactive = makeReactiveStore(store);
 * const docs = $derived(reactive.query("token"));
 * ```
 */
export function makeReactiveStore(store: DocumentStore) {
  const subscribe = createSubscriber((update) => store.subscribe(update));
  return {
    query(docType: string): WireDocument[] {
      subscribe();
      return store.query(docType);
    },
    get(id: string): WireDocument | undefined {
      subscribe();
      return store.get(id);
    },
  };
}
