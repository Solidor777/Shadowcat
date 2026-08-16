import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import App from "./App.svelte";
import * as api from "./lib/api";
import { WorldSession } from "./lib/worldSession.svelte";

// Stub the entry package: assert the shell renders it for pre-world routes without
// exercising entry's internals (covered by @shadowcat/module-entry's own tests).
vi.mock("@shadowcat/module-entry", async () => {
  const { default: Stub } = await import("./__fixtures__/EntryStub.svelte");
  return { Entry: Stub };
});

afterEach(() => vi.restoreAllMocks());

// `currentRoute()`'s backing state is module-scope $state, so it PERSISTS across every
// test in this suite (module caching — it is not re-initialized per test). A
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
