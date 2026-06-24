import { test, expect, vi, afterEach } from "vitest";
import * as api from "./asset-rest";

afterEach(() => vi.restoreAllMocks());

function mockFetch(status: number, body?: unknown) {
  return vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), { status }),
  );
}

test("uploadAsset POSTs multipart FormData and returns the asset", async () => {
  const asset = { id: "a1", world_id: "w1", version: 1 };
  const f = mockFetch(200, asset);
  const file = new File([new Uint8Array([1, 2, 3])], "x.png", { type: "image/png" });
  const out = await api.uploadAsset("w1", file);
  expect(out).toEqual(asset);
  const [url, init] = f.mock.calls[0];
  expect(url).toBe("/api/worlds/w1/assets");
  expect((init as RequestInit).method).toBe("POST");
  expect((init as RequestInit).body).toBeInstanceOf(FormData);
});

test("listAssets GETs the per-world list", async () => {
  mockFetch(200, [{ id: "a1" }]);
  expect(await api.listAssets("w1")).toHaveLength(1);
});

test("deleteAsset throws on a non-ok status", async () => {
  mockFetch(403);
  await expect(api.deleteAsset("a1")).rejects.toThrow();
});
