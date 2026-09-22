// @vitest-environment node
// Pure eviction-policy tests: no Pixi, no jsdom — see `texture-cache.ts`'s own module doc.
import { describe, it, expect, vi } from "vitest";
import { TextureCacheManager } from "./texture-cache";

describe("TextureCacheManager", () => {
  it("tracks total bytes across acquired entries", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 100, 0);
    cache.acquire("b", 200, 0);
    expect(cache.totalBytes()).toBe(300);
  });

  it("acquiring the same url twice increments its ref count without double-counting bytes", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 100, 0);
    cache.acquire("a", 100, 1);
    expect(cache.totalBytes()).toBe(100);
    const unload = vi.fn();
    cache.release("a", 2);
    // Still referenced once more: a sweep even with a zero budget must not evict it.
    cache.sweep(3, 0, Infinity, unload);
    expect(unload).not.toHaveBeenCalled();
  });

  it("never evicts an entry with a positive ref count, regardless of budget", () => {
    const cache = new TextureCacheManager();
    cache.acquire("live", 1_000_000, 0);
    const unload = vi.fn();
    cache.sweep(1000, 0, Infinity, unload);
    expect(unload).not.toHaveBeenCalled();
    expect(cache.has("live")).toBe(true);
  });

  it("evicts least-recently-used unreferenced entries first when over budget", () => {
    const cache = new TextureCacheManager();
    cache.acquire("old", 100, 0);
    cache.release("old", 0);
    cache.acquire("newer", 100, 10);
    cache.release("newer", 10);
    const unload = vi.fn();
    // Budget only fits one entry; "old" was used earlier so it evicts first.
    cache.sweep(20, 100, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("old");
    expect(cache.has("old")).toBe(false);
    expect(cache.has("newer")).toBe(true);
  });

  it("keeps evicting LRU-first until under budget, never touching referenced entries", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 100, 0);
    cache.release("a", 0);
    cache.acquire("b", 100, 1);
    cache.release("b", 1);
    cache.acquire("c", 100, 2); // still referenced
    const unload = vi.fn();
    cache.sweep(10, 100, Infinity, unload);
    expect(unload.mock.calls.map((c) => c[0])).toEqual(["a", "b"]);
    expect(cache.has("c")).toBe(true);
    expect(cache.totalBytes()).toBe(100);
  });

  it("does not evict when total bytes are within budget", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 50, 0);
    cache.release("a", 0);
    const unload = vi.fn();
    cache.sweep(10, 1000, Infinity, unload);
    expect(unload).not.toHaveBeenCalled();
  });

  it("TTL-sweeps an unreferenced entry idle past the threshold even under budget", () => {
    const cache = new TextureCacheManager();
    cache.acquire("stale", 10, 0);
    cache.release("stale", 0);
    const unload = vi.fn();
    cache.sweep(1000, 1_000_000, 500, unload); // well under budget, but idle 1000ms > 500ms ttl
    expect(unload).toHaveBeenCalledExactlyOnceWith("stale");
  });

  it("touch refreshes last-used so a TTL sweep does not evict a recently-reused entry", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 10, 0);
    cache.release("a", 0);
    cache.touch("a", 900);
    const unload = vi.fn();
    cache.sweep(1000, 1_000_000, 500, unload); // idle only 100ms since touch
    expect(unload).not.toHaveBeenCalled();
  });

  it("forgets an entry once unloaded — a later has() reports false", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 10, 0);
    cache.release("a", 0);
    cache.sweep(1, 0, Infinity, () => {});
    expect(cache.has("a")).toBe(false);
    expect(cache.totalBytes()).toBe(0);
  });

  it("release before any acquire is a no-op (never underflows into a phantom entry)", () => {
    const cache = new TextureCacheManager();
    expect(() => cache.release("never-acquired", 0)).not.toThrow();
    expect(cache.has("never-acquired")).toBe(false);
  });

  it("release floors the ref count at zero: an extra release cannot make a live entry evictable early then unevictable again", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 10, 0);
    cache.release("a", 0);
    cache.release("a", 0); // extra release beyond acquisitions
    const unload = vi.fn();
    cache.sweep(1, 0, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });

  it("register creates an entry at refCount 0 — a caller who never acquires it still leaves it evictable", () => {
    const cache = new TextureCacheManager();
    cache.register("a", 10, 0);
    expect(cache.has("a")).toBe(true);
    const unload = vi.fn();
    cache.sweep(1, 0, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });

  it("register on an already-tracked url is a no-op — it never clobbers an existing refCount or bytes", () => {
    const cache = new TextureCacheManager();
    cache.acquire("a", 10, 0);
    cache.register("a", 999_999, 5); // must not overwrite bytes or reset refCount
    expect(cache.totalBytes()).toBe(10);
    const unload = vi.fn();
    cache.sweep(1, 0, Infinity, unload); // still referenced: must not evict
    expect(unload).not.toHaveBeenCalled();
  });

  it("a NEW caller that loads-then-discards a texture through the loader alone still leaves it evictable — the central registration duty, driven only through register/beginLoad/endLoad with no per-site acquire/release", () => {
    const cache = new TextureCacheManager();
    // Mirrors what a loader wrapper does around an await: mark in-flight, then on completion
    // register (never acquire — this caller discards the result immediately) and end the load.
    cache.beginLoad("a");
    cache.endLoad("a");
    cache.register("a", 10, 0);
    expect(cache.has("a")).toBe(true);
    const unload = vi.fn();
    cache.sweep(1, 0, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });

  it("sweep never evicts a url with a load in flight, even at refCount 0 and past budget/TTL", () => {
    const cache = new TextureCacheManager();
    cache.register("a", 10, 0);
    cache.beginLoad("a"); // a second, concurrent load of the same url is still outstanding
    const unload = vi.fn();
    cache.sweep(1000, 0, 0, unload); // over budget AND past any TTL
    expect(unload).not.toHaveBeenCalled();
    expect(cache.has("a")).toBe(true);
  });

  it("sweep evicts once the in-flight load ends, restoring ordinary refCount-0 eviction", () => {
    const cache = new TextureCacheManager();
    cache.register("a", 10, 0);
    cache.beginLoad("a");
    cache.endLoad("a");
    const unload = vi.fn();
    cache.sweep(1000, 0, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });

  it("endLoad without a matching beginLoad is a no-op (never underflows into a negative count)", () => {
    const cache = new TextureCacheManager();
    expect(() => cache.endLoad("never-begun")).not.toThrow();
    cache.register("a", 10, 0);
    cache.endLoad("a"); // no beginLoad for "a" either — must not affect sweep eligibility
    const unload = vi.fn();
    cache.sweep(1, 0, Infinity, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });

  it("endLoad decrements a multi-load in-flight count rather than clearing it outright", () => {
    const cache = new TextureCacheManager();
    cache.register("a", 10, 0);
    cache.beginLoad("a");
    cache.beginLoad("a"); // two concurrent loads of the same url
    cache.endLoad("a"); // only one settles
    const unload = vi.fn();
    cache.sweep(1000, 0, 0, unload); // still one load outstanding
    expect(unload).not.toHaveBeenCalled();
    cache.endLoad("a"); // the second settles
    cache.sweep(1000, 0, 0, unload);
    expect(unload).toHaveBeenCalledExactlyOnceWith("a");
  });
});
