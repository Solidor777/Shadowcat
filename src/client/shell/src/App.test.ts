import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import App from "./App.svelte";
import * as api from "./lib/api";

afterEach(() => vi.restoreAllMocks());

test("uninitialized server routes to Setup", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: false });
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  expect(await screen.findByText("Create the admin account")).toBeTruthy();
});

test("initialized + unauthenticated routes to Login", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
});

test("authenticated routes to WorldSelect", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: null }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  render(App);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});

test("auto-enters the saved lastWorld on load", async () => {
  // Stub WebSocket so the session's connect attempt does not crash jsdom; it never
  // opens, so the session stays "connecting" and the Table shows "Connecting…".
  vi.stubGlobal("WebSocket", class { addEventListener() {} send() {} close() {} } as unknown);
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "w1" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([{ id: "w1", name: "W", role: "gm" }]);
  render(App);
  expect(await screen.findByText("Connecting…")).toBeTruthy();
  vi.unstubAllGlobals();
});

test("falls back to world-select when lastWorld is no longer accessible", async () => {
  vi.spyOn(api, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "gone" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([]); // "gone" not present
  render(App);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});
