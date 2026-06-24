import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import App from "./App.svelte";
import * as api from "./lib/api";

// Stub the entry package: assert the shell renders it for pre-world routes without
// exercising entry's internals (covered by @shadowcat/module-entry's own tests).
vi.mock("@shadowcat/module-entry", async () => {
  const { default: Stub } = await import("./__fixtures__/EntryStub.svelte");
  return { Entry: Stub };
});

afterEach(() => vi.restoreAllMocks());

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
