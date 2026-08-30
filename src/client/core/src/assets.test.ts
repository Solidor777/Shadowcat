import { describe, it, expect } from "vitest";
import { AssetResolver } from "./assets";
import type { Asset } from "@shadowcat/types";

/** A minimal, structurally-complete `Asset` fixture for `reconcile` tests — only `id` and
 * `version` matter to `AssetResolver`, but the full shape keeps the fixture assignable to the
 * generated wire type without a cast. */
function asset(id: string, version: number): Asset {
  return {
    id,
    world_id: "w1",
    storage_key: `w1/${id}`,
    original_name: "x.png",
    content_type: "image/png",
    byte_size: 1n,
    created_by: null,
    created_at: 0n,
    version: BigInt(version),
    folder_id: null,
    tags: [],
    derived_tags: [],
    width: null,
    height: null,
    has_alpha: false,
    animated: false,
    original_content_type: "image/png",
    original_byte_size: 1n,
    original_retained: false,
    conversion_note: null,
  };
}

describe("AssetResolver", () => {
  it("resolves a uuid to the serve URL", () => {
    const r = new AssetResolver();
    expect(r.url("abc")).toBe("/api/assets/abc");
  });

  it("after replace, the URL changes (cache-bust) so the new bytes load", () => {
    const r = new AssetResolver();
    const before = r.url("abc");
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 1 });
    expect(r.url("abc")).not.toBe(before);
  });

  it("after delete, the uuid resolves to the placeholder", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 1 });
    expect(r.url("abc")).toBe(r.placeholder());
  });

  it("onAssetChanged sets the cache-bust query to the frame's authoritative version, not a relative bump", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 5 });
    expect(r.url("abc")).toBe("/api/assets/abc?v=5");
  });

  it("out-of-order onAssetChanged delivery converges on the highest version seen, never regressing", () => {
    // The server broadcasts in commit order over one connection, so this ordering shouldn't
    // occur in practice; the resolver still guards against it rather than trusting arrival
    // order, since a regression here would re-introduce a cache-bust key that goes BACKWARDS
    // (the same class of staleness reconciliation exists to close).
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 5 });
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 3 });
    expect(r.url("abc")).toBe("/api/assets/abc?v=5");
  });

  it("reconcile updates revs from a listing's authoritative version even with no onAssetChanged ever received", () => {
    // Proves the self-healing path: a resolver that never got a bump still converges once a
    // listing carrying the true version is reconciled.
    const r = new AssetResolver();
    r.reconcile([asset("abc", 7)]);
    expect(r.url("abc")).toBe("/api/assets/abc?v=7");
  });

  it("reconcile never regresses a version already held from a live AssetChanged frame", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 9 });
    r.reconcile([asset("abc", 4)]); // a stale listing snapshot, e.g. raced against a later replace
    expect(r.url("abc")).toBe("/api/assets/abc?v=9");
  });

  it("reconcile clears a deleted marker when the listing carries a genuinely newer version", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 1 });
    expect(r.url("abc")).toBe(r.placeholder());
    r.reconcile([asset("abc", 2)]);
    expect(r.url("abc")).toBe("/api/assets/abc?v=2");
  });

  it("a reconcile carrying the SAME version as a delete it raced does not resurrect the asset", () => {
    // The exact race: a listing request in flight when a delete lands. The delete's broadcast
    // and the stale listing snapshot report the same version (deletion removes the row; it does
    // not bump its version), so the version comparison correctly rejects the stale reconcile.
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 5 });
    expect(r.url("abc")).toBe(r.placeholder());
    r.reconcile([asset("abc", 5)]);
    expect(r.url("abc")).toBe(r.placeholder());
  });

  it("an ordinary delete at the same version already held is honored, not rejected as stale", () => {
    // The standard flow: a resolver already knows the asset's current version (via reconcile, the
    // ordinary case for any surface that lists assets), then the eventual AssetChanged delete
    // notice arrives carrying that SAME version (deletion never bumps the version column). This
    // is the authoritative delete signal, not a stale write, and must be honored.
    const r = new AssetResolver();
    r.reconcile([asset("abc", 1)]);
    expect(r.url("abc")).toBe("/api/assets/abc?v=1");
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 1 });
    expect(r.url("abc")).toBe(r.placeholder());
  });

  it("a late-arriving stale delete notice is rejected once a newer version is already held", () => {
    // e.g. the uuid was reused by a later upload at a higher version before a delayed delete
    // frame (reporting an older version) finally arrives. The delete must not win against a
    // version this resolver has already advanced past.
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 5 });
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 3 });
    expect(r.url("abc")).toBe("/api/assets/abc?v=5");
  });

  it("url with a variant carries the variant and the rev, in that order", () => {
    const r = new AssetResolver();
    expect(r.url("abc", "thumb")).toBe("/api/assets/abc?variant=thumb");
    r.onAssetChanged({ uuid: "abc", op: "replaced", version: 4 });
    expect(r.url("abc", "preview")).toBe("/api/assets/abc?variant=preview&v=4");
    expect(r.url("abc")).toBe("/api/assets/abc?v=4");
    r.onAssetChanged({ uuid: "abc", op: "deleted", version: 4 });
    expect(r.url("abc", "thumb")).toBe(r.placeholder());
  });

  it("created and moved notify listing listeners but never change a URL; replaced does the inverse", () => {
    const r = new AssetResolver();
    const seen: [string, string][] = [];
    const stop = r.onListingInvalidated((uuid, op) => seen.push([uuid, op]));
    r.onAssetChanged({ uuid: "n", op: "created", version: 1 });
    const before = r.url("n");
    r.onAssetChanged({ uuid: "n", op: "moved", version: 1 });
    expect(r.url("n")).toBe(before);
    r.onAssetChanged({ uuid: "n", op: "replaced", version: 2 });
    expect(r.url("n")).not.toBe(before);
    r.onAssetChanged({ uuid: "n", op: "deleted", version: 2 });
    expect(seen).toEqual([
      ["n", "created"],
      ["n", "moved"],
      ["n", "deleted"],
    ]);
    stop();
    r.onAssetChanged({ uuid: "n", op: "moved", version: 2 });
    expect(seen).toHaveLength(3);
  });
});
