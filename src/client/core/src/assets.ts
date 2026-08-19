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
  /** The asset's authoritative version at the time of the mutation: the bumped version for
   * `op: "replaced"`, or the version the row held immediately before removal for
   * `op: "deleted"` — a real ordering token in both cases. */
  version: number;
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

  /** Adopts an observed `version` for a uuid if and only if it is strictly higher than any
   * version already held, gating BOTH state changes a mutation notice can carry — the
   * cache-busting revision and the deleted marker — behind the same comparison, so a stale
   * write (`version <= current`) is a no-op across the board rather than a partial one. Shared
   * by `onAssetChanged` and `reconcile`, the two touchpoints that can observe a version for a
   * uuid: an out-of-band mutation notice, and a listing snapshot that may predate one.
   * @param uuid The asset's stable uuid.
   * @param version The observed authoritative version.
   * @param isDeleted Whether the observation reports the asset as deleted at that version.
   * @example
   * ```
   * // internal helper; not part of the public API
   * this.adoptVersion("00000000-0000-0000-0000-000000000001", 2, false);
   * ```
   */
  private adoptVersion(uuid: string, version: number, isDeleted: boolean): void {
    const current = this.revs.get(uuid) ?? -1;
    if (version <= current) return;
    this.revs.set(uuid, version);
    if (isDeleted) this.deleted.add(uuid);
    else this.deleted.delete(uuid);
  }

  /** Invalidate a uuid in response to an AssetChanged frame, routed through `adoptVersion` so a
   * stale or out-of-order frame (`msg.version` not higher than any version already held) is a
   * no-op — including for `op: "deleted"`, which now carries a real ordering token rather than
   * discarding all version memory for the uuid (see the class doc: a frame that never arrives at
   * all is not fixed by this method; `reconcile` closes that gap).
   * @param msg The broadcast frame; adopted via `adoptVersion` against `msg.version`.
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
    this.adoptVersion(msg.uuid, msg.version, msg.op === "deleted");
  }

  /** Reconciles `revs`/`deleted` against a listing's authoritative records — the self-healing
   * path for an `AssetChanged` frame this connection never received at all (see the class doc).
   * For each record, routes through `adoptVersion`: a listing whose snapshot predates a delete
   * this resolver already observed carries the SAME version the delete broadcast did, so the
   * comparison rejects it as stale rather than resurrecting the asset; a listing whose snapshot
   * postdates the delete (a genuinely higher version, e.g. a re-upload reusing the id) clears the
   * `deleted` marker and adopts the new version.
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
      // Asset.version is ts-rs's bigint mapping of the server's i64; JSON.parse (the actual
      // decode path for a listAssets response) always yields number, so this narrows back to
      // the same representation `revs`/onAssetChanged already use.
      this.adoptVersion(a.id, Number(a.version), false);
    }
  }
}
