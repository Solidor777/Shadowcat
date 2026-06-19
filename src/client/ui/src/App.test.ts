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
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  render(App);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});
