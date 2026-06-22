/** Op carried by an out-of-band AssetChanged frame. */
export type AssetOp = "replaced" | "deleted";

/**
 * Resolves asset UUIDs to serve URLs and reacts to out-of-band AssetChanged
 * notices. The server's ETag handles HTTP caching; a monotonic per-uuid `rev`
 * counter cache-busts the URL on replace so a fresh request (and thus ETag
 * revalidation) happens. Deleted uuids resolve to the placeholder.
 */
export class AssetResolver {
  private revs = new Map<string, number>();
  private deleted = new Set<string>();

  /** A neutral 1×1 transparent placeholder. */
  placeholder(): string {
    return "data:image/gif;base64,R0lGODlhAQABAAAAACwAAAAAAQABAAA=";
  }

  url(uuid: string): string {
    if (this.deleted.has(uuid)) return this.placeholder();
    const rev = this.revs.get(uuid);
    return rev === undefined ? `/api/assets/${uuid}` : `/api/assets/${uuid}?v=${rev}`;
  }

  /** Invalidate a uuid in response to an AssetChanged frame. */
  onAssetChanged(msg: { uuid: string; op: AssetOp }): void {
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
