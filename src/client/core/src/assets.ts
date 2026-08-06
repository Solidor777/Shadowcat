/** Op carried by an out-of-band AssetChanged frame. */
export type AssetOp = "replaced" | "deleted";

/**
 * Resolves asset UUIDs to serve URLs and reacts to out-of-band AssetChanged
 * notices. The server's ETag handles HTTP caching; a monotonic per-uuid `rev`
 * counter cache-busts the URL on replace so a fresh request (and thus ETag
 * revalidation) happens. Deleted uuids resolve to the placeholder.
 *
 * KNOWN DEFECT (`docs/OPEN_BUGS.md`, the AssetChanged entry): `revs` is
 * client-local and bumped only by `onAssetChanged` — it never reads the
 * asset's server-side `version`. A connection that misses an
 * `AssetChanged{replaced}` frame (an ordinary reconnect suffices, since
 * `Room::subscribe` returns a receiver at the channel's current tail and the
 * frame is never in the resync ring) keeps a byte-identical `url()` result
 * forever; no new request is ever issued, so nothing self-heals until a page
 * reload.
 */
export class AssetResolver {
  /** Per-uuid cache-busting revision, incremented only by `onAssetChanged` —
   * see the class doc's KNOWN DEFECT for why a missed frame leaves this stale. */
  private revs = new Map<string, number>();
  /** Uuids known-deleted; `url()` resolves any member to `placeholder()`. */
  private deleted = new Set<string>();

  /** A neutral 1×1 transparent placeholder.
   * @returns A `data:` URI for a 1×1 transparent GIF.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.placeholder();
   * ```
   */
  placeholder(): string {
    return "data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=";
  }

  /** Resolves an asset uuid to a serve URL, cache-busted by the current `rev`
   * so a replace forces a fresh request (and thus ETag revalidation); a
   * deleted uuid resolves to `placeholder()`. Does NOT read the asset's
   * server-side `version` — see the class doc's KNOWN DEFECT: a connection
   * that missed the `AssetChanged` frame for a replace returns the same URL
   * forever, never revalidating.
   * @param uuid The asset's stable uuid.
   * @returns The `/api/assets/{uuid}` URL, or the placeholder if deleted.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.url("00000000-0000-0000-0000-000000000001");
   * ```
   */
  url(uuid: string): string {
    if (this.deleted.has(uuid)) return this.placeholder();
    const rev = this.revs.get(uuid);
    return rev === undefined ? `/api/assets/${uuid}` : `/api/assets/${uuid}?v=${rev}`;
  }

  /** Invalidate a uuid in response to an AssetChanged frame. This is the ONLY
   * way `revs`/`deleted` ever change — a frame this connection never
   * receives (see the class doc's KNOWN DEFECT) leaves both permanently
   * stale for that uuid.
   * @param msg The broadcast frame.
   * @param msg.uuid The affected asset's uuid.
   * @param msg.op `"replaced"` bumps the cache-busting revision; `"deleted"` switches `url()` to the placeholder.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.onAssetChanged({ uuid: "00000000-0000-0000-0000-000000000001", op: "replaced" });
   * ```
   */
  onAssetChanged(msg: {
    /** The affected asset's stable uuid. */
    uuid: string;
    /** `"replaced"` bumps the cache-busting revision; `"deleted"` switches `url()` to the placeholder. */
    op: AssetOp;
  }): void {
    if (msg.op === "deleted") {
      this.deleted.add(msg.uuid);
      this.revs.delete(msg.uuid);
      return;
    }
    // replaced: drop any delete marker and bump the cache-bust revision.
    this.deleted.delete(msg.uuid);
    this.revs.set(msg.uuid, (this.revs.get(msg.uuid) ?? 0) + 1);
  }
}
