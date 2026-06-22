import { describe, it, expect } from "vitest";
import { AssetResolver } from "./assets";

describe("AssetResolver", () => {
  it("resolves a uuid to the serve URL", () => {
    const r = new AssetResolver();
    expect(r.url("abc")).toBe("/api/assets/abc");
  });

  it("after replace, the URL changes (cache-bust) so the new bytes load", () => {
    const r = new AssetResolver();
    const before = r.url("abc");
    r.onAssetChanged({ uuid: "abc", op: "replaced" });
    expect(r.url("abc")).not.toBe(before);
  });

  it("after delete, the uuid resolves to the placeholder", () => {
    const r = new AssetResolver();
    r.onAssetChanged({ uuid: "abc", op: "deleted" });
    expect(r.url("abc")).toBe(r.placeholder());
  });
});
