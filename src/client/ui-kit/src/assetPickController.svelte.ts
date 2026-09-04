// Asset pick-mode orchestration. Holds one reactive pending pick request the
// asset-browser module's overlay renders; `AppContext.pickAsset` is the
// requester-facing wrapper the shell builds over `request`. Imports no module
// (seam boundary: the overlay reaches this only through AppContext).

/** Filters and arity a `pickAsset` caller asks the browser to open with. */
export interface PickAssetOptions {
  /** Restrict the browser's kind filter (`image` covers every raster/vector art pick). */
  kind?: "image" | "other";
  /** Pre-applied tag filter chips (all-of). */
  tags?: string[];
  /** Ordered multi-pick: the promise resolves `string[]` in pick order. */
  multiple?: boolean;
}

/** `PickAssetOptions` with the ordered multi-pick arity selected. */
export interface PickAssetMultiple extends PickAssetOptions {
  /** Discriminates the array-returning `pickAsset` overload. */
  multiple: true;
}

/** A pick in progress; `resolve` settles the requester's promise. */
export interface PendingPick {
  /** The requester's filters/arity, preset on the browser when it opens. */
  opts: PickAssetOptions;
  /** Settles the requester's promise (ids in pick order, `null` = cancel). */
  resolve: (ids: string[] | null) => void;
}

/**
 * Stable-ref, mutate-in-place holder of the one active pick request
 * (AppContext values must never be reassigned `$state` — consumers hold the
 * reference). One pick at a time: a new `request` cancels the previous.
 */
export class AssetPickController {
  /** The active pick, or `null`; the overlay renders iff non-null. */
  pending = $state<PendingPick | null>(null);

  /**
   * Opens a pick and resolves with the chosen ids (in pick order) or `null`
   * on cancel. A second request while one is open resolves the first with
   * `null` and replaces it.
   * @param opts - Filters/arity preset on the browser.
   * @returns The picked asset ids, or `null` if cancelled.
   * @example
   * ```ts
   * const c = new AssetPickController();
   * const picked = c.request({ kind: "image", multiple: true });
   * c.settle(["a", "b"]);
   * void picked; // resolves ["a", "b"]
   * ```
   */
  request(opts: PickAssetOptions = {}): Promise<string[] | null> {
    this.pending?.resolve(null);
    return new Promise((resolve) => {
      this.pending = { opts, resolve };
    });
  }

  /**
   * Settles the active pick. Clears `pending` BEFORE resolving so a
   * re-entrant `request` from the resolution path is never clobbered.
   * @param ids - Picked ids in pick order, or `null` for cancel.
   * @example
   * ```ts
   * const c = new AssetPickController();
   * const picked = c.request();
   * c.settle(null); // cancel: `picked` resolves null
   * void picked;
   * ```
   */
  settle(ids: string[] | null): void {
    const p = this.pending;
    this.pending = null;
    p?.resolve(ids);
  }
}
