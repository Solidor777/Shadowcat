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
  expect(put.mock.calls.at(-1)?.[0].global?.lastWorld).toBe("w2");
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
  expect(put.mock.calls.at(-1)?.[0].global?.locale).toBe("zz");
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
  expect(put.mock.calls.at(-1)?.[0].worlds?.w1?.panelLayout).toBe(blob);
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
  expect(put.mock.calls.at(-1)?.[0].worlds?.w1?.chatRead).toBe(blob);
});

test("a world-slice change persists ONLY the touched key within that world's slice", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {
      w1: { chatRead: { general: 1 } }, // a different key owner already stored on w1
      wOther: { chatRead: { general: 1 } },
    },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setPanelLayout("w1", { version: 1 });
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.worlds).toEqual({ w1: { panelLayout: { version: 1 } } });
  expect(patch?.global).toBeUndefined();
  expect(patch?.worlds?.wOther).toBeUndefined();
});

test("a global change persists ONLY the touched field within global", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null }, // `locale` untouched below
    worlds: { w1: { panelLayout: { version: 1 } } },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w2");
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.global).toEqual({ lastWorld: "w2" });
  expect(patch?.worlds).toBeUndefined();
});

test("a locale-only change persists locale without lastWorld", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: "w1" }, // `lastWorld` untouched below
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  i18n.setLocale("zz");
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.global).toEqual({ locale: "zz" });
});

test("a failed persist re-marks the lost slice so a later flush retries it alongside a new one", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi
    .spyOn(api, "putUiState")
    .mockRejectedValueOnce(new Error("network"))
    .mockResolvedValue();
  await loadSessionState();
  setPanelLayout("w1", { version: 1 }); // leading-edge persist rejects
  await flushSessionState(); // lets the rejection's catch re-mark w1.panelLayout
  setChatRead("w2", { general: 1 }); // a new mutation while w1 is still dirty from the failure
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.worlds).toEqual({
    w1: { panelLayout: { version: 1 } },
    w2: { chatRead: { general: 1 } },
  });
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

test("a second persist attempt while one is unresolved is deferred, not raced", async () => {
  vi.useFakeTimers();
  try {
    vi.spyOn(api, "getUiState").mockResolvedValue({
      global: { locale: "en", lastWorld: null },
      worlds: {},
    });
    let resolveFirst!: () => void;
    const put = vi
      .spyOn(api, "putUiState")
      .mockImplementationOnce(() => new Promise<void>((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValue(undefined);
    await loadSessionState();
    // A preceding real-timer test can leave its own cooldown `timer` still armed for up to
    // COOLDOWN_MS after that test ends (neither `loadSessionState` nor `flushOnUnload` cancels
    // it); with `timer` non-null, `schedulePersist` would take its cooldown branch instead of
    // firing the leading edge this test depends on. `flushSessionState` cancels that leftover
    // timer; dirty tracking is already empty at this point (`loadSessionState` just cleared it),
    // so this is a no-op PUT and does not consume the mocked hanging first call below.
    await flushSessionState();
    put.mockClear();

    setLastWorld("w1"); // leading edge: persist() starts immediately, putUiState #1 pending
    expect(put).toHaveBeenCalledTimes(1);

    // Still inside the cooldown window: this only sets pendingDuringCooldown, no new persist() yet.
    await vi.advanceTimersByTimeAsync(100);
    setLastWorld("w2");
    expect(put).toHaveBeenCalledTimes(1);

    // Cooldown elapses (400ms remaining of the 500ms window) while putUiState #1 is STILL pending
    // (resolveFirst has not been called). Without the fix, this would fire a second, overlapping
    // putUiState call right here.
    await vi.advanceTimersByTimeAsync(400);
    expect(put).toHaveBeenCalledTimes(1); // deferred, not raced

    // Settle the first PUT; the deferred attempt runs automatically and picks up w2.
    resolveFirst();
    await vi.waitFor(() => expect(put).toHaveBeenCalledTimes(2));
    expect(put.mock.calls[1]?.[0].global?.lastWorld).toBe("w2");
  } finally {
    vi.useRealTimers();
  }
});

test("flushOnUnload re-marks the slice on a rejected keepalive PUT so a later flush retries it", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi
    .spyOn(api, "putUiState")
    .mockResolvedValueOnce() // leading-edge persist for "w1"
    .mockRejectedValueOnce(new Error("unload")) // the keepalive flush rejects
    .mockResolvedValue(); // the later retry succeeds
  await loadSessionState();
  setLastWorld("w1"); // leading-edge persist, cooldown timer armed
  setLastWorld("w2"); // lands during cooldown → pending, not yet written
  flushOnUnload(); // keepalive PUT rejects
  await flushSessionState(); // no dirty slice yet; lets the rejection's catch re-mark lastWorld
  await flushSessionState(); // this time lastWorld is dirty again → retries
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.global).toEqual({ lastWorld: "w2" });
});
