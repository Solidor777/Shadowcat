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

test("queryAssets encodes every filter and always selects the page form", async () => {
  const f = mockFetch(200, { items: [], next_cursor: null });
  await api.queryAssets("w1", {
    folder: "root",
    recursive: true,
    tags: ["hero", "map"],
    kind: "image",
    name: "crypt",
    nameRegex: "^cr.pt$",
    sort: "size",
    limit: 5,
    cursor: "abc",
  });
  const url = new URL(String(f.mock.calls[0][0]), "http://x");
  expect(url.pathname).toBe("/api/worlds/w1/assets");
  expect(Object.fromEntries(url.searchParams)).toEqual({
    folder: "root",
    recursive: "true",
    tags: "hero,map",
    kind: "image",
    name: "crypt",
    name_regex: "^cr.pt$",
    sort: "size",
    limit: "5",
    cursor: "abc",
  });
  const g = mockFetch(200, { items: [], next_cursor: null });
  await api.queryAssets("w1", {});
  expect(String(g.mock.calls.at(-1)?.[0])).toBe("/api/worlds/w1/assets?limit=200");
});

test("patchAsset PATCHes JSON and surfaces the server's error text", async () => {
  const f = mockFetch(200, { id: "a1" });
  await api.patchAsset("a1", { name: "x.png", folder_id: null });
  const [url, init] = f.mock.calls[0];
  expect(url).toBe("/api/assets/a1");
  expect((init as RequestInit).method).toBe("PATCH");
  expect(JSON.parse((init as RequestInit).body as string)).toEqual({ name: "x.png", folder_id: null });
  mockFetch(422, { error: "empty tag" });
  await expect(api.patchAsset("a1", { tags: [""] })).rejects.toThrow("patch failed: 422 empty tag");
});

test("bulkPatchAssets POSTs to the world's bulk route", async () => {
  const f = mockFetch(200, [{ id: "a1" }, { id: "a2" }]);
  const out = await api.bulkPatchAssets("w1", {
    ids: ["a1", "a2"],
    folder_id: "f1",
    add_tags: ["map"],
    remove_tags: [],
  });
  expect(out).toHaveLength(2);
  expect(f.mock.calls[0][0]).toBe("/api/worlds/w1/assets/bulk");
  expect((f.mock.calls[0][1] as RequestInit).method).toBe("POST");
});

test("reconvertAsset POSTs and originalUrl is a plain path", async () => {
  const f = mockFetch(200, { id: "a1", version: 2 });
  expect((await api.reconvertAsset("a1")).version).toBe(2);
  expect(f.mock.calls[0][0]).toBe("/api/assets/a1/reconvert");
  expect(api.originalUrl("a1")).toBe("/api/assets/a1/original");
});
