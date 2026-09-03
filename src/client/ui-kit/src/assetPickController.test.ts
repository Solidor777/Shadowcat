import { describe, it, expect } from "vitest";
import { AssetPickController } from "./assetPickController.svelte";

describe("AssetPickController", () => {
  it("resolves the request with the settled ids in pick order", async () => {
    const c = new AssetPickController();
    const p = c.request({ kind: "image" });
    expect(c.pending?.opts).toEqual({ kind: "image" });
    c.settle(["a", "b"]);
    await expect(p).resolves.toEqual(["a", "b"]);
    expect(c.pending).toBeNull();
  });

  it("resolves null on cancel", async () => {
    const c = new AssetPickController();
    const p = c.request();
    c.settle(null);
    await expect(p).resolves.toBeNull();
    expect(c.pending).toBeNull();
  });

  it("a second request cancels the first with null", async () => {
    const c = new AssetPickController();
    const first = c.request();
    const second = c.request({ multiple: true });
    await expect(first).resolves.toBeNull();
    expect(c.pending?.opts).toEqual({ multiple: true });
    c.settle(["x"]);
    await expect(second).resolves.toEqual(["x"]);
  });

  it("clears pending before resolving, so a re-entrant request is not clobbered", async () => {
    const c = new AssetPickController();
    let reentrant: Promise<string[] | null> | undefined;
    const outer = c.request().then((ids) => {
      // Runs from settle(): a new request opened here must survive.
      reentrant = c.request({ kind: "other" });
      return ids;
    });
    c.settle(["a"]);
    await expect(outer).resolves.toEqual(["a"]);
    expect(c.pending?.opts).toEqual({ kind: "other" });
    c.settle(["z"]);
    await expect(reentrant).resolves.toEqual(["z"]);
  });
});
