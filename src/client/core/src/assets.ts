import type { Asset } from "@shadowcat/types";

/** Op carried by an out-of-band AssetChanged frame. `created` and `moved` (name / folder /
 * tags) change what a LISTING shows, never what a URL serves; `replaced` and `deleted` change
 * the bytes behind a URL. */
export type AssetOp = "created" | "replaced" | "moved" | "deleted";

/** A derivative size class servable via `?variant=`: `thumb` (≤128px) or `preview` (≤512px). */
export type AssetVariant = "thumb" | "preview";

/** An out-of-band asset mutation notice; carries no seq. Shared by
 * `AssetResolver.onAssetChanged` and `WsClientHandlers.onAssetChanged` — both consume the
 * identical wire shape. */
export interface AssetChangedNotice {
  /** The changed asset's uuid. */
  uuid: string;
  /** What happened to the asset. */
  op: AssetOp;
  /** The asset's authoritative version at the time of the mutation: the bumped version for
   * `op: "replaced"`, the version the row held immediately before removal for
   * `op: "deleted"`, `1` for `op: "created"`, and the unchanged current version for
   * `op: "moved"` — a real ordering token in every case. */
  version: number;
}

/** Callback for `AssetResolver.onListingInvalidated`. */
export type ListingInvalidatedHandler = (uuid: string, op: AssetOp) => void;

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
  /** Listeners told when a listing went stale (`created` / `moved` / `deleted`). */
  private listingListeners = new Set<ListingInvalidatedHandler>();

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
   * deleted uuid resolves to `placeholder()`. With `variant`, the URL names a
   * derivative (`?variant=thumb|preview`) — same ETag basis as the canonical,
   * so the same `rev` busts it.
   * @param uuid The asset's stable uuid.
   * @param variant A derivative size class; absent = the canonical file.
   * @returns The `/api/assets/{uuid}` URL, or the placeholder if deleted.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * resolver.url("00000000-0000-0000-0000-000000000001");
   * ```
   */
  url(uuid: string, variant?: AssetVariant): string {
    if (this.deleted.has(uuid)) return this.placeholder();
    const rev = this.revs.get(uuid);
    const params: string[] = [];
    if (variant !== undefined) params.push(`variant=${variant}`);
    if (rev !== undefined) params.push(`v=${rev}`);
    return params.length === 0 ? `/api/assets/${uuid}` : `/api/assets/${uuid}?${params.join("&")}`;
  }

  /** Subscribe to listing-invalidating notices (`created`, `moved`, `deleted`): the
   * assets a listing shows, or where it files them, changed — refetch the listing.
   * `replaced` is not reported here; it changes bytes, not listings, and `url()`
   * already reflects it.
   * @param handler Called with the uuid and op of each such notice.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { AssetResolver } from "@shadowcat/core";
   *
   * const resolver = new AssetResolver();
   * const stop = resolver.onListingInvalidated((uuid, op) => console.info(uuid, op));
   * stop();
   * ```
   */
  onListingInvalidated(handler: ListingInvalidatedHandler): () => void {
    this.listingListeners.add(handler);
    return () => {
      this.listingListeners.delete(handler);
    };
  }

  /** Adopts an observed `version` for a uuid, gating both state changes a mutation notice can
   * carry — the cache-busting revision and the deleted marker — behind one comparison whose
   * strictness depends on `isDeleted`: a delete transition adopts at `version >= current`,
   * because deletion never bumps the version column, so a delete notice reporting exactly the
   * version this resolver already holds is the ORDINARY case, not staleness, and must be
   * honored; every other adoption (always via `reconcile`, which never itself carries a delete
   * signal) requires `version > current` strictly, because a reconcile snapshot may predate a
   * delete already adopted at that same version and must not resurrect it. Shared by
   * `onAssetChanged` and `reconcile`, the two touchpoints that can observe a version for a uuid:
   * an out-of-band mutation notice, and a listing snapshot that may predate one.
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
    if (isDeleted) {
      const isStale = version < current;
      if (isStale) return;
      this.revs.set(uuid, version);
      this.deleted.add(uuid);
    } else {
      const isStaleOrSame = version <= current;
      if (isStaleOrSame) return;
      this.revs.set(uuid, version);
      this.deleted.delete(uuid);
    }
  }

  /** Invalidate a uuid in response to an AssetChanged frame, routed through `adoptVersion` so a
   * stale or out-of-order frame is a no-op: a `deleted` frame is rejected only if `msg.version`
   * is BELOW any version already held (equal is the ordinary case, since deletion never bumps
   * the version column, and is honored), while a `replaced` frame is rejected unless
   * `msg.version` is strictly higher. `op: "deleted"` carries a real ordering token rather than
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
    // `created` (version 1) and `moved` (unchanged version) never lower the
    // rev, and `created` clears a stale deleted marker only if the version
    // is genuinely newer — the same monotonic rule every adoption follows.
    this.adoptVersion(msg.uuid, msg.version, msg.op === "deleted");
    if (msg.op !== "replaced") {
      for (const listener of this.listingListeners) listener(msg.uuid, msg.op);
    }
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
   *     folder_id: null,
   *     tags: [],
   *     derived_tags: [],
   *     width: null,
   *     height: null,
   *     has_alpha: false,
   *     animated: false,
   *     original_content_type: "image/png",
   *     original_byte_size: 1n,
   *     original_retained: false,
   *     conversion_note: null,
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
