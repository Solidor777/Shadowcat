import { test, expect, vi, afterEach } from "vitest";
import * as api from "./api";

afterEach(() => vi.restoreAllMocks());

function mockFetch(status: number, body?: unknown) {
  return vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), { status }),
  );
}

test("getMe returns null on 401, the body on 200", async () => {
  mockFetch(401);
  expect(await api.getMe()).toBeNull();
  mockFetch(200, { id: "u1", username: "a", server_role: "user" });
  expect((await api.getMe())?.id).toBe("u1");
});

test("listWorlds returns the world array", async () => {
  mockFetch(200, [{ id: "w1", name: "W", role: "gm" }]);
  const worlds = await api.listWorlds();
  expect(worlds[0].name).toBe("W");
});

test("getUiState normalizes an empty server blob to defaults", async () => {
  mockFetch(200, {});
  const s = await api.getUiState();
  expect(s).toEqual({ global: { locale: "en", lastWorld: null }, worlds: {} });
});

test("getUiState passes through a stored blob", async () => {
  mockFetch(200, { global: { locale: "en", lastWorld: "w1" }, worlds: { w1: { activeTab: "settings" } } });
  const s = await api.getUiState();
  expect(s.global.lastWorld).toBe("w1");
});

test("putUiState PUTs the blob", async () => {
  const f = mockFetch(204);
  await api.putUiState({ global: { locale: "en", lastWorld: null }, worlds: {} });
  expect(f).toHaveBeenCalledWith("/api/me/ui-state", expect.objectContaining({ method: "PUT" }));
});

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
