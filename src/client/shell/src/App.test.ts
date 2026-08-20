import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import App from "./App.svelte";
import * as api from "./lib/api";
import { WorldSession } from "./lib/worldSession.svelte";
import type { WorldEntry } from "@shadowcat/types";

// Stub the entry package: assert the shell renders it for pre-world routes without
// exercising entry's internals (covered by @shadowcat/module-entry's own tests).
vi.mock("@shadowcat/module-entry", async () => {
  const { default: Stub } = await import("./__fixtures__/EntryStub.svelte");
  return { Entry: Stub };
});

afterEach(() => vi.restoreAllMocks());

// `currentRoute()`'s backing state is module-scope $state, so it PERSISTS across every
// subsequent test (module caching — it is not re-initialized per test). A
// prior test's `navigate()` call (e.g. entering a world sets the hash to
// `#/world/<id>`) otherwise leaks into the next test, which now matters
// because boot() reads `currentRoute()`. Reset the hash — and manually
// dispatch `hashchange`, since a jsdom `location.hash` assignment does not
// reliably re-run the router's own listener within a microtask-timed test —
// so every test starts from a known bare route.
afterEach(() => {
  location.hash = "";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
});

test("renders the entry package when not auto-entering a world", async () => {
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();
});

test("auto-enters the saved lastWorld on load", async () => {
  // Stub WebSocket so the session's connect attempt does not crash jsdom; it never
  // opens, so the session stays "connecting" and the Table shows "Connecting…".
  vi.stubGlobal("WebSocket", class { addEventListener() {} send() {} close() {} } as unknown);
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "w1" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([{ id: "w1", name: "W", role: "gm" }]);
  render(App);
  expect(await screen.findByText("Connecting…")).toBeTruthy();
  vi.unstubAllGlobals();
});

test("falls back to entry when the saved lastWorld is no longer accessible", async () => {
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "gone" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([]); // "gone" not present
  render(App);
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();
});

test("a world-route deep link with a failing backend falls back to entry, not a stuck stale Connecting frame", async () => {
  // End-to-end regression for navigate()'s synchronous currentRoute() update (see
  // route.svelte.ts's own doc comment and route.test.ts's discriminating unit test for the
  // mechanism): boot()'s catch calls navigate({name:"login"}) on a getMe() failure, and this
  // asserts the app actually reaches <Entry> rather than the stale `{:else if route.name ===
  // "world"}` branch (which renders "Connecting…" with no session).
  location.hash = "#/world/abc-123";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  vi.spyOn(api, "getMe").mockRejectedValue(new Error("network"));
  render(App);
  // A rejecting getMe() exhausts withRetry's real (unmocked) 3-attempt/500-1500ms-jittered
  // backoff before boot()'s catch navigates away — comfortably under 5s worst case.
  expect(await screen.findByTestId("entry-stub", {}, { timeout: 5000 })).toBeTruthy();
  expect(screen.queryByText("Connecting…")).toBeNull();
});

test("reload on a valid world route enters THAT world, not a different valid lastWorld", async () => {
  // Pins the App↔currentRoute wiring (not just resolveBootWorld's pure logic):
  // a stale-fix regression here (boot() reverting to lastWorld-first) would
  // still pass resolveBootWorld's own unit tests unchanged.
  vi.stubGlobal("WebSocket", class { addEventListener() {} send() {} close() {} } as unknown);
  location.hash = "#/world/route-world";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  const enterSpy = vi.spyOn(WorldSession.prototype, "enter").mockResolvedValue(undefined);
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "last-world" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([
    { id: "route-world", name: "Route World", role: "gm" },
    { id: "last-world", name: "Last World", role: "gm" },
  ]);
  render(App);
  await screen.findByText("Connecting…");
  expect(enterSpy).toHaveBeenCalledWith("route-world");
  expect(enterSpy).not.toHaveBeenCalledWith("last-world");
  vi.unstubAllGlobals();
});

test("boot shows a still-trying message after the threshold when the backend is slow", async () => {
  vi.useFakeTimers();
  try {
    let resolveMe!: (me: Awaited<ReturnType<typeof api.getMe>>) => void;
    vi.spyOn(api, "getMe").mockReturnValue(
      new Promise((resolve) => {
        resolveMe = resolve;
      }),
    );
    render(App);
    expect(screen.getByText("Loading…")).toBeTruthy();
    await vi.advanceTimersByTimeAsync(8_000);
    expect(screen.getByText("Still trying…")).toBeTruthy();
    resolveMe(null);
    await vi.runOnlyPendingTimersAsync();
  } finally {
    vi.useRealTimers();
  }
});

test("boot gives up at the deadline and a late-resolving fetch afterward is a no-op", async () => {
  vi.useFakeTimers();
  try {
    const enterSpy = vi.spyOn(WorldSession.prototype, "enter").mockResolvedValue(undefined);
    vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
    vi.spyOn(api, "getUiState").mockResolvedValue({
      global: { locale: "en", lastWorld: "w1" },
      worlds: {},
    });
    vi.spyOn(api, "putUiState").mockResolvedValue();
    let resolveWorlds!: (worlds: WorldEntry[]) => void;
    vi.spyOn(api, "listWorlds").mockReturnValue(
      new Promise((resolve) => {
        resolveWorlds = resolve;
      }),
    );
    render(App);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(await screen.findByTestId("entry-stub")).toBeTruthy();
    // getMe/getUiState resolved promptly (not delayed) and boot() is sitting at the listWorlds
    // await — past the first two `if (abandoned) return;` guards, with `abandoned` still false at
    // the time this await was reached — when the deadline fires. Resolving listWorlds() now, with
    // a world matching lastWorld, exercises SPECIFICALLY the guard immediately before
    // resolveBootWorld/enterWorld: if it were removed, this would call enterWorld("w1") and
    // render "Connecting…" instead of leaving entry-stub in place.
    resolveWorlds([{ id: "w1", name: "W", role: "gm" }]);
    await vi.runOnlyPendingTimersAsync();
    expect(screen.getByTestId("entry-stub")).toBeTruthy();
    expect(enterSpy).not.toHaveBeenCalled();
  } finally {
    vi.useRealTimers();
  }
});

test("a hash change during the listWorlds await is honored, not the route captured before it", async () => {
  vi.stubGlobal("WebSocket", class { addEventListener() {} send() {} close() {} } as unknown);
  location.hash = "#/world/stale-world";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  const enterSpy = vi.spyOn(WorldSession.prototype, "enter").mockResolvedValue(undefined);
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: null }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  let resolveWorlds!: (worlds: WorldEntry[]) => void;
  vi.spyOn(api, "listWorlds").mockReturnValue(
    new Promise((resolve) => {
      resolveWorlds = resolve;
    }),
  );
  render(App);
  await screen.findByText("Loading…");
  location.hash = "#/world/fresh-world";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  resolveWorlds([
    { id: "stale-world", name: "Stale", role: "gm" },
    { id: "fresh-world", name: "Fresh", role: "gm" },
  ]);
  await screen.findByText("Connecting…");
  expect(enterSpy).toHaveBeenCalledWith("fresh-world");
  expect(enterSpy).not.toHaveBeenCalledWith("stale-world");
  vi.unstubAllGlobals();
});
