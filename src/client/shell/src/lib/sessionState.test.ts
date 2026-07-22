import { test, expect, vi, afterEach, beforeEach } from "vitest";
import * as api from "./api";
import { i18n } from "@shadowcat/ui-kit";
import {
  loadSessionState,
  getSessionState,
  setLastWorld,
  getPanelLayout,
  setPanelLayout,
  getChatRead,
  setChatRead,
  flushSessionState,
  flushOnUnload,
} from "./sessionState.svelte";

beforeEach(() => i18n.setLocale("en"));
afterEach(() => vi.restoreAllMocks());

test("load applies the saved locale and exposes the blob", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: "w1" },
    worlds: {},
  });
  const s = await loadSessionState();
  expect(s.global.lastWorld).toBe("w1");
  expect(i18n.locale).toBe("en");
  expect(getSessionState().global.lastWorld).toBe("w1");
});

test("setLastWorld updates state and persists (debounced)", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w2");
  expect(getSessionState().global.lastWorld).toBe("w2");
  await flushSessionState();
  expect(put).toHaveBeenCalled();
  expect(put.mock.calls.at(-1)?.[0].global.lastWorld).toBe("w2");
});

test("a locale change persists the new locale", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  i18n.setLocale("zz");
  await flushSessionState();
  expect(put.mock.calls.at(-1)?.[0].global.locale).toBe("zz");
});

test("getPanelLayout returns null for a world with no recorded state", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  await loadSessionState();
  expect(getPanelLayout("w1")).toBeNull();
});

test("setPanelLayout records the blob per-world and schedules a debounced persist", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  const blob = { version: 1 };
  setPanelLayout("w1", blob);
  expect(getPanelLayout("w1")).toBe(blob);
  expect(getPanelLayout("w2")).toBeNull();
  await flushSessionState();
  expect(put).toHaveBeenCalled();
  expect(put.mock.calls.at(-1)?.[0].worlds.w1?.panelLayout).toBe(blob);
});

test("getChatRead returns null for a world with no recorded state", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  await loadSessionState();
  expect(getChatRead("w1")).toBeNull();
});

test("setChatRead records the blob per-world and schedules a debounced persist", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  const blob = { general: { createdAt: 1, id: "m1" } };
  setChatRead("w1", blob);
  expect(getChatRead("w1")).toBe(blob);
  expect(getChatRead("w2")).toBeNull();
  await flushSessionState();
  expect(put).toHaveBeenCalled();
  expect(put.mock.calls.at(-1)?.[0].worlds.w1?.chatRead).toBe(blob);
});

test("flushOnUnload keepalive-persists a change made during the cooldown", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w1"); // leading-edge persist, cooldown timer armed
  setLastWorld("w2"); // lands during cooldown → pending, not yet written
  flushOnUnload();
  expect(put).toHaveBeenLastCalledWith(
    expect.objectContaining({ global: expect.objectContaining({ lastWorld: "w2" }) }),
    { keepalive: true },
  );
});
