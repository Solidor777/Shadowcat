// Synchronous-resolution seam for a single asset's pipeline metadata (in particular
// AssetMeta.sheet), for a caller that has never listed the world's assets (AssetResolver
// caches only version + deleted flag, never full metadata). Warmed eagerly by the host
// (Stage.svelte) for every emitter's asset id, and awaited before a one-shot's first
// vfxAssets(id) lookup, so the render layer's own resolver call is always synchronous.
import type { Asset } from "@shadowcat/types";
import { getAssetMeta } from "./asset-rest";

/**
 * In-memory, per-world-session cache of `Asset` metadata (never bytes), keyed by asset id.
 * `AssetResolver`'s existing cache tracks only version + deleted-ness for URL cache-busting;
 * this cache exists purely so a synchronous resolver (`resolveVfxSource`) has something to
 * read once the async fetch below has completed.
 * @example
 * ```ts
 * const cache = new AssetMetaCache();
 * await cache.warm("00000000-0000-0000-0000-000000000001");
 * cache.get("00000000-0000-0000-0000-000000000001");
 * ```
 */
export class AssetMetaCache {
  /** Resolved metadata, keyed by asset id. */
  #entries = new Map<string, Asset>();
  /** In-flight fetches, keyed by asset id — de-dupes a concurrent double-warm. */
  #pending = new Map<string, Promise<Asset | null>>();

  /**
   * The cached metadata for `id`, or `null` if never warmed (or the warm failed).
   * @param id The asset id.
   * @returns The cached `Asset`, or `null`.
   * @example
   * ```ts
   * const cache = new AssetMetaCache();
   * cache.get("00000000-0000-0000-0000-000000000001"); // null
   * ```
   */
  get(id: string): Asset | null {
    return this.#entries.get(id) ?? null;
  }

  /**
   * Fetches `id`'s metadata (via `getAssetMeta`) and caches it; a concurrent call for the
   * same id shares one in-flight fetch. A fetch failure (network error, 404) resolves to
   * `null` and leaves the cache unset for `id` — a later warm retries.
   * @param id The asset id to warm.
   * @returns The fetched `Asset`, or `null` on failure.
   * @example
   * ```ts
   * const cache = new AssetMetaCache();
   * await cache.warm("00000000-0000-0000-0000-000000000001");
   * ```
   */
  async warm(id: string): Promise<Asset | null> {
    const cached = this.#entries.get(id);
    if (cached) return cached;
    const pending = this.#pending.get(id);
    if (pending) return pending;
    const p = getAssetMeta(id)
      .then((a) => {
        this.#entries.set(id, a);
        return a;
      })
      .catch(() => null)
      .finally(() => this.#pending.delete(id));
    this.#pending.set(id, p);
    return p;
  }
}
