import { test, expect, vi, afterEach } from "vitest";
import * as api from "./api";

afterEach(() => vi.restoreAllMocks());

function mockFetch(status: number, body?: unknown) {
  return vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), { status }),
  );
}

test("getConfig returns the parsed config", async () => {
  mockFetch(200, { initialized: true });
  expect(await api.getConfig()).toEqual({ initialized: true });
});

test("getMe returns null on 401, the body on 200", async () => {
  mockFetch(401);
  expect(await api.getMe()).toBeNull();
  mockFetch(200, { id: "u1", username: "a", server_role: "user" });
  expect((await api.getMe())?.id).toBe("u1");
});

test("login returns true on 204, false on 401", async () => {
  mockFetch(204);
  expect(await api.login("a", "b")).toBe(true);
  mockFetch(401);
  expect(await api.login("a", "x")).toBe(false);
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
