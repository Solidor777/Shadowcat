// @vitest-environment node
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Asset } from "@shadowcat/types";
import { AssetMetaCache } from "./asset-meta";

const ID = "00000000-0000-0000-0000-000000000001";

function fakeAsset(id: string): Asset {
  return { id } as unknown as Asset;
}

function okResponse(asset: Asset): Response {
  return new Response(JSON.stringify(asset), { status: 200 });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("AssetMetaCache", () => {
  it("warm caches a successful fetch and get returns it", async () => {
    const asset = fakeAsset(ID);
    const fetchMock = vi.fn().mockResolvedValue(okResponse(asset));
    vi.stubGlobal("fetch", fetchMock);
    const cache = new AssetMetaCache();
    expect(cache.get(ID)).toBeNull();
    const got = await cache.warm(ID);
    expect(got).toEqual(asset);
    expect(cache.get(ID)).toEqual(asset);
    expect(fetchMock).toHaveBeenCalledWith(`/api/assets/${ID}/meta`);
  });

  it("a concurrent double-warm shares one in-flight fetch", async () => {
    const asset = fakeAsset(ID);
    let resolveFetch: (r: Response) => void = () => {};
    const fetchMock = vi.fn().mockImplementation(
      () =>
        new Promise<Response>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const cache = new AssetMetaCache();
    const p1 = cache.warm(ID);
    const p2 = cache.warm(ID);
    resolveFetch(okResponse(asset));
    const [a, b] = await Promise.all([p1, p2]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(a).toEqual(asset);
    expect(b).toEqual(asset);
  });

  it("a failing fetch resolves warm to null and leaves get returning null", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 404 }));
    vi.stubGlobal("fetch", fetchMock);
    const cache = new AssetMetaCache();
    const got = await cache.warm(ID);
    expect(got).toBeNull();
    expect(cache.get(ID)).toBeNull();
  });

  it("a later warm after a failure retries", async () => {
    const asset = fakeAsset(ID);
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response("nope", { status: 404 }))
      .mockResolvedValueOnce(okResponse(asset));
    vi.stubGlobal("fetch", fetchMock);
    const cache = new AssetMetaCache();
    expect(await cache.warm(ID)).toBeNull();
    expect(await cache.warm(ID)).toEqual(asset);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(cache.get(ID)).toEqual(asset);
  });

  it("a cached entry short-circuits without a new fetch", async () => {
    const asset = fakeAsset(ID);
    const fetchMock = vi.fn().mockResolvedValue(okResponse(asset));
    vi.stubGlobal("fetch", fetchMock);
    const cache = new AssetMetaCache();
    await cache.warm(ID);
    await cache.warm(ID);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
