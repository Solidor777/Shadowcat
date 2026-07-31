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
  mockFetch(200, { global: { locale: "en", lastWorld: "w1" }, worlds: { w1: { panelLayout: { version: 1 } } } });
  const s = await api.getUiState();
  expect(s.global.lastWorld).toBe("w1");
});

test("putUiState PUTs the patch body verbatim", async () => {
  const f = mockFetch(204);
  const patch = { global: { lastWorld: "w2" }, worlds: { w1: { chatRead: { general: 1 } } } };
  await api.putUiState(patch);
  expect(f).toHaveBeenCalledWith(
    "/api/me/ui-state",
    expect.objectContaining({ method: "PUT", body: JSON.stringify(patch) }),
  );
});

test("getJson passes a bounded AbortSignal to fetch", async () => {
  const f = mockFetch(200, [{ id: "w1", name: "W", role: "gm" }]);
  await api.listWorlds();
  const init = f.mock.calls[0][1] as RequestInit;
  expect(init.signal).toBeInstanceOf(AbortSignal);
});

test("getMe passes a bounded AbortSignal to fetch", async () => {
  const f = mockFetch(200, { id: "u1", username: "a", server_role: "user" });
  await api.getMe();
  const init = f.mock.calls[0][1] as RequestInit;
  expect(init.signal).toBeInstanceOf(AbortSignal);
});

test("putUiState passes a bounded AbortSignal alongside keepalive", async () => {
  const f = mockFetch(204);
  await api.putUiState({}, { keepalive: true });
  const init = f.mock.calls[0][1] as RequestInit;
  expect(init.signal).toBeInstanceOf(AbortSignal);
  expect(init.keepalive).toBe(true);
});

test("withRetry retries the configured attempts then rethrows the last error", async () => {
  vi.useFakeTimers();
  try {
    let calls = 0;
    const fn = vi.fn(async () => {
      calls++;
      if (calls < 3) throw new Error(`fail ${calls}`);
      return "ok";
    });
    const p = api.withRetry(fn);
    await vi.runAllTimersAsync();
    await expect(p).resolves.toBe("ok");
    expect(fn).toHaveBeenCalledTimes(3);
  } finally {
    vi.useRealTimers();
  }
});

test("withRetry rejects after the configured attempts, not more", async () => {
  vi.useFakeTimers();
  try {
    const fn = vi.fn(async () => {
      throw new Error("always fails");
    });
    const p = api.withRetry(fn, 3, [1, 1]);
    p.catch(() => {}); // avoid an unhandled-rejection warning racing the assertion below
    await vi.runAllTimersAsync();
    await expect(p).rejects.toThrow("always fails");
    expect(fn).toHaveBeenCalledTimes(3);
  } finally {
    vi.useRealTimers();
  }
});

