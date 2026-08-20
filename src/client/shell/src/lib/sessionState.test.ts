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
  resetSessionState,
  pruneStaleWorlds,
} from "./sessionState.svelte";

// Module-level state (loaded/dirty/the cooldown timer) persists across tests otherwise — a
// real-timer test's cooldown can still be armed up to COOLDOWN_MS into the next test.
beforeEach(() => {
  i18n.setLocale("en");
  resetSessionState();
});
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

test("a failed persist's re-marked slice is retried once the in-flight attempt settles, not lost", async () => {
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
  // The in-flight-PUT ordering guard's queued retry runs (and this await genuinely waits for
  // it, not just the failed first attempt) BEFORE this line returns — the retry succeeds on
  // its own, independent of any later mutation, rather than waiting to be combined with one.
  await flushSessionState();
  expect(put).toHaveBeenCalledTimes(2);
  expect(put.mock.calls[1]?.[0].worlds).toEqual({ w1: { panelLayout: { version: 1 } } });

  // A later, independent mutation sends its own patch normally.
  setChatRead("w2", { general: 1 });
  await flushSessionState();
  expect(put).toHaveBeenCalledTimes(3);
  expect(put.mock.calls[2]?.[0].worlds).toEqual({ w2: { chatRead: { general: 1 } } });
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

test("resetSessionState clears loaded, dirty tracking, and any pending timer", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setLastWorld("w1"); // schedules a persist (leading edge fires immediately + starts cooldown)
  resetSessionState();

  // A mutation attempted after reset must not schedule a persist — `loaded` is false again,
  // mirroring the state before the first loadSessionState() call.
  const callsBeforeReattempt = put.mock.calls.length;
  setLastWorld("w2");
  await new Promise((r) => setTimeout(r, 10));
  expect(put.mock.calls.length).toBe(callsBeforeReattempt); // no new persist scheduled while unloaded

  // A subsequent loadSessionState() (re-login) restores normal operation.
  await loadSessionState();
  setLastWorld("w3");
  await flushSessionState();
  expect(put.mock.calls.at(-1)?.[0].global?.lastWorld).toBe("w3");
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

test("pruneStaleWorlds is a no-op when every stored world id is still a member", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { w1: { panelLayout: { version: 1 } } },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  pruneStaleWorlds(["w1"]);
  expect(getSessionState().worlds).toEqual({ w1: { panelLayout: { version: 1 } } });
  expect(put).not.toHaveBeenCalled();
});

test("pruneStaleWorlds removes a stale entry from session state immediately", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { w1: { panelLayout: { version: 1 } }, w2: { chatRead: { general: 1 } } },
  });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  pruneStaleWorlds(["w2"]); // w1 is stale
  expect(getSessionState().worlds).toEqual({ w2: { chatRead: { general: 1 } } });
});

test("pruneStaleWorlds triggers a persist sending a null removal for the stale id", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { w1: { panelLayout: { version: 1 } } },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  pruneStaleWorlds([]); // w1 is stale
  await flushSessionState();
  expect(put).toHaveBeenCalled();
  expect(put.mock.calls.at(-1)?.[0].worlds).toEqual({ w1: null });
});

test("pruneStaleWorlds removing a world with pending per-key dirty state sends ONLY the null removal", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {},
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  setPanelLayout("w1", { version: 1 }); // leading-edge persist fires + starts cooldown
  pruneStaleWorlds([]); // w1 pruned while its panelLayout write is still pending/in-flight
  await flushSessionState();
  const patch = put.mock.calls.at(-1)?.[0];
  expect(patch?.worlds).toEqual({ w1: null });
});

test("pruneStaleWorlds retries a failed removal via remarkDirty", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: { w1: { panelLayout: { version: 1 } } },
  });
  const put = vi
    .spyOn(api, "putUiState")
    .mockRejectedValueOnce(new Error("network"))
    .mockResolvedValue();
  await loadSessionState();
  pruneStaleWorlds([]); // leading-edge persist rejects
  await flushSessionState();
  expect(put).toHaveBeenCalledTimes(2);
  expect(put.mock.calls[1]?.[0].worlds).toEqual({ w1: null });

  // A second successful flush retries the same null removal.
  await flushSessionState();
  expect(put).toHaveBeenCalledTimes(2); // nothing dirty left after the successful retry above
});

test("flushOnUnload flushes a pending pruneStaleWorlds removal", async () => {
  vi.spyOn(api, "getUiState").mockResolvedValue({
    global: { locale: "en", lastWorld: null },
    worlds: {
      w1: { panelLayout: { version: 1 } },
      w2: { panelLayout: { version: 1 } },
      w3: { panelLayout: { version: 1 } },
    },
  });
  const put = vi.spyOn(api, "putUiState").mockResolvedValue();
  await loadSessionState();
  pruneStaleWorlds(["w2", "w3"]); // removes w1: leading-edge persist fires + starts cooldown
  pruneStaleWorlds(["w3"]); // removes w2: lands during cooldown → pending, not yet written
  flushOnUnload();
  expect(put).toHaveBeenLastCalledWith(expect.objectContaining({ worlds: { w2: null } }), {
    keepalive: true,
  });
});
