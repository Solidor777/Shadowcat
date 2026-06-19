import { test, expect, vi, afterEach, beforeEach } from "vitest";
import * as api from "./api";
import { i18n } from "./i18n.svelte";
import {
  loadSessionState,
  getSessionState,
  setLastWorld,
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
