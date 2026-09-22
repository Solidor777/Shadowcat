// Pure GPU-texture eviction policy, no Pixi dependency and no I/O — `pixi-backend.ts` is the
// sole caller, injecting real byte estimates and issuing the actual `Assets.unload` calls this
// module only decides the TIMING and TARGET of. Kept framework-agnostic so the eviction
// decision (LRU-under-budget, TTL-idle) is unit-testable without a GL context.
//
// Multi-scene GM roam means a token's art from a scene the GM is no longer viewing has no live
// sprite referencing it, yet PixiJS's own `Assets` cache keeps it GPU-resident for the tab's
// life unless something explicitly unloads it — this is that something.

/** One tracked texture source's bookkeeping. */
interface TextureCacheEntry {
  /** Approximate GPU-resident byte size (e.g. `width * height * 4` for RGBA8). */
  bytes: number;
  /** The last time (ms, caller's clock) this entry was acquired or touched. */
  lastUsed: number;
  /** How many live consumers (sprites/animated frames/the background) currently display this
   * source. `sweep` never evicts an entry with `refCount > 0` — this is the invariant closing
   * the "never unload a texture a live sprite still references" requirement. An entry CAN exist
   * at `refCount === 0` before any caller acquires it (`register` creates one this way at load
   * completion, so a discarded load result is still tracked); an in-flight load of that same url
   * is instead kept safe by `#inFlight`, which `sweep` checks independently of `refCount`. */
  refCount: number;
}

/** Tracks approximate byte size, last-use time, and live-reference count per texture source
 * (keyed by serve URL), and decides which entries `sweep` may evict. Carries no knowledge of
 * PixiJS `Texture`/`Assets` objects — `pixi-backend.ts` estimates bytes and performs the actual
 * `Assets.unload` call via the `unload` callback `sweep` invokes. */
export class TextureCacheManager {
  /** Every tracked source, by url. */
  #entries = new Map<string, TextureCacheEntry>();

  /** Per-url count of loads currently in flight (`beginLoad` called, `endLoad` not yet called
   * for that same load) — `sweep` skips any url with a nonzero count here regardless of
   * `refCount`, closing the window between a loser's `register` (refCount 0, evictable) and the
   * eventual winner's `acquire`: without this, a sweep landing in that window could unload a
   * texture a still-outstanding load is about to display. */
  #inFlight = new Map<string, number>();

  /** Registers one live consumer of `url`, creating the entry on first acquisition (or
   * refreshing its `lastUsed`/`bytes` on a later one — `bytes` from a later acquisition of an
   * already-tracked url is authoritative, since a source's true size cannot change but a
   * caller's own estimate might be recomputed more precisely at a later call site).
   * @param url The texture source's serve URL.
   * @param bytes Approximate GPU-resident byte size.
   * @param now The current time, caller's clock (ms).
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.acquire("/api/assets/1", 1024, 0);
   * ```
   */
  acquire(url: string, bytes: number, now: number): void {
    // `register` is the sole `TextureCacheEntry` construction site; ensure one exists (a no-op
    // when `url` is already tracked) and then mutate it in place, rather than building a second
    // entry literal here.
    this.register(url, bytes, now);
    const entry = this.#entries.get(url);
    if (!entry) return;
    entry.bytes = bytes;
    entry.lastUsed = now;
    entry.refCount += 1;
  }

  /** Releases one live consumer of `url` (the exact inverse of one `acquire` call) — the entry
   * stays tracked (still evictable by `sweep` once its ref count reaches 0) rather than being
   * removed immediately, so its size/last-use bookkeeping survives until a sweep actually
   * decides to unload it. A `url` with no tracked entry, or whose ref count is already 0, is a
   * no-op (never underflows into a negative count or a phantom entry).
   * @param url The texture source's serve URL.
   * @param now The current time, caller's clock (ms) — refreshes `lastUsed` so the idle clock
   * for TTL purposes starts from the moment of release, not the original acquisition.
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.acquire("/api/assets/1", 1024, 0);
   * cache.release("/api/assets/1", 5);
   * ```
   */
  release(url: string, now: number): void {
    const entry = this.#entries.get(url);
    if (!entry) return;
    entry.refCount = Math.max(0, entry.refCount - 1);
    entry.lastUsed = now;
  }

  /** Registers a url that has just finished loading, exactly once at the LOAD SITE — creates the
   * entry at `refCount: 0` (immediately sweep-eligible, subject only to TTL/budget) when no entry
   * exists yet. This is what makes a load whose caller never follows up with `acquire` (the
   * caller lost a race, or discarded the result) still become tracked and evictable, instead of
   * landing in PixiJS's own `Assets` cache untracked for the tab's life. A url already tracked —
   * by a concurrent load, an earlier load, or a caller's already-live reference — is left
   * completely untouched: this must never clobber an existing `refCount` or overwrite a fresher
   * `bytes` estimate with a stale one.
   * @param url The texture source's serve URL.
   * @param bytes Approximate GPU-resident byte size, used only when creating a new entry.
   * @param now The current time, caller's clock (ms), used only when creating a new entry.
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.register("/api/assets/1", 1024, 0); // evictable immediately: no live consumer yet
   * ```
   */
  register(url: string, bytes: number, now: number): void {
    if (this.#entries.has(url)) return;
    this.#entries.set(url, { bytes, lastUsed: now, refCount: 0 });
  }

  /** Marks one load of `url` as in flight — `sweep` will not evict `url` while any load remains
   * in flight, regardless of `refCount` or idle time. Must be paired with exactly one later
   * `endLoad(url)` call for the same load (a caller awaiting the load wraps the await).
   * ASSUMPTION: a load that never settles (neither resolves nor rejects, so `endLoad` is never
   * called) pins that url against eviction indefinitely — there is no timeout here that forces
   * an in-flight count back to zero; a caller's load promise is expected to eventually settle on
   * its own.
   * @param url The texture source's serve URL being loaded.
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.beginLoad("/api/assets/1");
   * cache.endLoad("/api/assets/1");
   * ```
   */
  beginLoad(url: string): void {
    this.#inFlight.set(url, (this.#inFlight.get(url) ?? 0) + 1);
  }

  /** The exact inverse of one `beginLoad(url)` call — decrements the in-flight count, deleting
   * the tracking entry entirely once it reaches zero (so `sweep`'s check is a simple `.has`). A
   * `url` with no in-flight count is a no-op (never underflows into a negative count).
   * @param url The texture source's serve URL whose load just settled (resolved or rejected).
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.beginLoad("/api/assets/1");
   * cache.endLoad("/api/assets/1");
   * ```
   */
  endLoad(url: string): void {
    const n = this.#inFlight.get(url);
    if (n === undefined) return;
    if (n <= 1) this.#inFlight.delete(url);
    else this.#inFlight.set(url, n - 1);
  }

  /** Refreshes `url`'s last-used time without changing its ref count — for a cache HIT that
   * does not itself acquire a new reference (e.g. a texture reused verbatim by an unchanged
   * node). A `url` with no tracked entry is a no-op.
   * @param url The texture source's serve URL.
   * @param now The current time, caller's clock (ms).
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.acquire("/api/assets/1", 1024, 0);
   * cache.touch("/api/assets/1", 5);
   * ```
   */
  touch(url: string, now: number): void {
    const entry = this.#entries.get(url);
    if (entry) entry.lastUsed = now;
  }

  /** Whether `url` is currently tracked (referenced or not).
   * @param url The texture source's serve URL.
   * @returns `true` if an entry exists.
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * new TextureCacheManager().has("/api/assets/1"); // false
   * ```
   */
  has(url: string): boolean {
    return this.#entries.has(url);
  }

  /** The sum of every tracked entry's `bytes` (referenced or not).
   * @returns The approximate total GPU-resident bytes this cache is tracking.
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * new TextureCacheManager().totalBytes(); // 0
   * ```
   */
  totalBytes(): number {
    let total = 0;
    for (const e of this.#entries.values()) total += e.bytes;
    return total;
  }

  /** Runs one eviction pass: first unloads every UNREFERENCED entry idle longer than `ttlMs`
   * (independent of budget — a long-idle texture is dropped even when GPU memory is otherwise
   * plentiful), then unloads unreferenced entries oldest-`lastUsed`-first while `totalBytes()`
   * exceeds `budgetBytes`. An entry with `refCount > 0` is skipped by BOTH passes regardless of
   * budget or idle time — this is what makes a live sprite's texture safe: it is never tracked
   * with a zero ref count while still needed. A url with a nonzero `beginLoad`/`endLoad` count is
   * ALSO skipped by both passes regardless of `refCount` — this is what makes an in-flight load
   * safe even in the window between a loser's `register` (refCount 0, otherwise evictable) and
   * the eventual winner's `acquire`.
   * @param now The current time, caller's clock (ms).
   * @param budgetBytes The byte budget; entries evict oldest-first while total exceeds this.
   * @param ttlMs Unreferenced entries idle longer than this are evicted regardless of budget.
   * @param unload Called once per evicted url (the caller's actual `Assets.unload`); the entry
   * is forgotten immediately after this returns, so a synchronous throw from `unload` still
   * leaves the cache's own bookkeeping consistent (the entry is removed either way — a leaked
   * GPU texture behind a swallowed error is worse than an inconsistent size estimate).
   * @example
   * ```ts
   * import { TextureCacheManager } from "@shadowcat/render";
   * const cache = new TextureCacheManager();
   * cache.acquire("/api/assets/1", 1024, 0);
   * cache.release("/api/assets/1", 0);
   * cache.sweep(10_000, 0, Infinity, (url) => { void url; });
   * ```
   */
  sweep(now: number, budgetBytes: number, ttlMs: number, unload: (url: string) => void): void {
    const evict = (url: string): void => {
      this.#entries.delete(url);
      unload(url);
    };
    for (const [url, entry] of this.#entries) {
      if (entry.refCount === 0 && !this.#inFlight.has(url) && now - entry.lastUsed > ttlMs) evict(url);
    }
    if (this.totalBytes() <= budgetBytes) return;
    const candidates = [...this.#entries.entries()]
      .filter(([url, e]) => e.refCount === 0 && !this.#inFlight.has(url))
      .sort((a, b) => a[1].lastUsed - b[1].lastUsed);
    for (const [url] of candidates) {
      if (this.totalBytes() <= budgetBytes) break;
      evict(url);
    }
  }
}
