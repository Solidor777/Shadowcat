import type { Asset } from "@shadowcat/types";

/** Op carried by an out-of-band AssetChanged frame. */
export type AssetOp = "replaced" | "deleted";

/** An out-of-band asset mutation notice (replace/delete); carries no seq. Shared by
 * `AssetResolver.onAssetChanged` and `WsClientHandlers.onAssetChanged` — both consume the
 * identical wire shape. */
export interface AssetChangedNotice {
  /** The changed asset's uuid. */
  uuid: string;
  /** Whether the asset's bytes were replaced or the asset was deleted. */
  op: AssetOp;
  /** The asset's authoritative version after the mutation: a number for `op: "replaced"`,
   * `null` for `op: "deleted"` (a deleted asset has no version). */
  version: number | null;
}

/**
 * Resolves asset UUIDs to serve URLs and reacts to out-of-band AssetChanged
 * notices. The server's ETag handles HTTP caching; the cache-bust key is the
 * asset's AUTHORITATIVE `version` — carried directly on `AssetChanged` frames
 * and, for a frame this connection never received (an ordinary reconnect
 * drops any `AssetChanged` broadcast in flight, since `Room::broadcast_aux`
 * is out-of-band and never replayed on resync), reconciled opportunistically
 * via `reconcile` from any touchpoint that already fetches full `Asset`
 * records (e.g. `listAssets`). Deleted uuids resolve to the placeholder.
 */
export class AssetResolver {
  /** Per-uuid cache-busting revision: the highest authoritative `version` this
   * resolver has observed for the uuid, via `onAssetChanged` or `reconcile`. */
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
   * deleted uuid resolves to `placeholder()`.
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

  /** Invalidate a uuid in response to an AssetChanged frame. `op: "replaced"` sets the
   * cache-busting revision to `msg.version` — the AUTHORITATIVE value, not a relative bump —
   * unless a higher version is already held (see the class doc: a frame that never arrives at
   * all is not fixed by this method; `reconcile` closes that gap). `op: "deleted"` switches
   * `url()` to the placeholder.
   * @param msg The broadcast frame; `op: "replaced"` sets the cache-busting revision to
   * `msg.version` (if higher than any version already held), `op: "deleted"` switches `url()`
   * to the placeholder.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.onAssetChanged({
   *   uuid: "00000000-0000-0000-0000-000000000001",
   *   op: "replaced",
   *   version: 2,
   * });
   * ```
   */
  onAssetChanged(msg: AssetChangedNotice): void {
    if (msg.op === "deleted") {
      this.deleted.add(msg.uuid);
      this.revs.delete(msg.uuid);
      return;
    }
    // replaced: drop any delete marker and adopt the authoritative version, never
    // regressing below a higher version already held (out-of-order delivery guard —
    // the server broadcasts in commit order over one connection, so this is defensive).
    this.deleted.delete(msg.uuid);
    const current = this.revs.get(msg.uuid) ?? -1;
    if (msg.version !== null && msg.version > current) this.revs.set(msg.uuid, msg.version);
  }

  /** Reconciles `revs`/`deleted` against a listing's authoritative records — the self-healing
   * path for an `AssetChanged` frame this connection never received at all (see the class doc).
   * For each record, adopts its `version` if higher than any version already held, and clears a
   * stale `deleted` marker for its id (an asset present in a fresh listing is definitionally not
   * deleted from this client's perspective, whatever a missed `AssetChanged{deleted}` frame might
   * have implied).
   * @param assets The listing's records (e.g. from `listAssets`).
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.reconcile([
   *   {
   *     id: "00000000-0000-0000-0000-000000000001",
   *     world_id: "00000000-0000-0000-0000-000000000002",
   *     storage_key: "k",
   *     original_name: "n",
   *     content_type: "image/png",
   *     byte_size: 1n,
   *     created_by: null,
   *     created_at: 0n,
   *     version: 4n,
   *   },
   * ]);
   * ```
   */
  reconcile(assets: readonly Asset[]): void {
    for (const a of assets) {
      this.deleted.delete(a.id);
      // Asset.version is ts-rs's bigint mapping of the server's i64; JSON.parse (the actual
      // decode path for a listAssets response) always yields number, so this narrows back to
      // the same representation `revs`/onAssetChanged already use.
      const version = Number(a.version);
      const current = this.revs.get(a.id) ?? -1;
      if (version > current) this.revs.set(a.id, version);
    }
  }
}
