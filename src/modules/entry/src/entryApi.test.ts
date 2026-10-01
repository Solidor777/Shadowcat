import { test, expect, vi, afterEach } from "vitest";
import * as api from "./entryApi";

afterEach(() => vi.restoreAllMocks());

function mockFetch(status: number, body?: unknown) {
  return vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(body === undefined ? null : JSON.stringify(body), { status }),
  );
}

test("getConfig returns the parsed config", async () => {
  const body = {
    initialized: true,
    tls: { enabled: true, source: "self-signed", https_origins: [], alpn: ["h2", "http/1.1"] },
  };
  mockFetch(200, body);
  expect(await api.getConfig()).toEqual(body);
});

test("a missing or malformed tls block leaves initialized readable and tls null", async () => {
  mockFetch(200, { initialized: false });
  expect(await api.getConfig()).toEqual({ initialized: false, tls: null });
  mockFetch(200, { initialized: false, tls: { enabled: "yes" } });
  expect(await api.getConfig()).toEqual({ initialized: false, tls: null });
});

test("getConfig rejects a body whose initialized flag is not a boolean", async () => {
  mockFetch(200, { tls: null });
  await expect(api.getConfig()).rejects.toThrow();
});

test("getMe returns null on 401, the id on 200", async () => {
  mockFetch(401);
  expect(await api.getMe()).toBeNull();
  mockFetch(200, { id: "u1" });
  expect((await api.getMe())?.id).toBe("u1");
});

test("login returns true on 204, false on 401", async () => {
  mockFetch(204);
  expect(await api.login("a", "b")).toBe(true);
  mockFetch(401);
  expect(await api.login("a", "x")).toBe(false);
});

test("setup reports ok and status", async () => {
  mockFetch(204);
  expect(await api.setup("a", "b")).toEqual({ ok: true, status: 204 });
  mockFetch(403);
  expect(await api.setup("a", "b", "bad")).toEqual({ ok: false, status: 403 });
});

test("listWorlds returns the world array", async () => {
  mockFetch(200, [{ id: "w1", name: "W", role: "gm" }]);
  const worlds = await api.listWorlds();
  expect(worlds[0].name).toBe("W");
});

test("createWorld returns the created world; throws on a non-ok status", async () => {
  mockFetch(200, { id: "w1", name: "W", role: "gm" });
  expect((await api.createWorld("W")).id).toBe("w1");
  mockFetch(500);
  await expect(api.createWorld("X")).rejects.toThrow();
});

test("deleteWorld issues DELETE and throws on failure", async () => {
  const fetchMock = mockFetch(204);
  await api.deleteWorld("w1");
  expect(fetchMock).toHaveBeenCalledWith("/api/worlds/w1", { method: "DELETE" });
  mockFetch(403);
  await expect(api.deleteWorld("w1")).rejects.toThrow();
});
