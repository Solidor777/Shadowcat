import { render, screen, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import Entry from "./Entry.svelte";
import * as entryApi from "./entryApi";

afterEach(() => vi.restoreAllMocks());

const props = { onAuthenticated: () => {}, onEnterWorld: () => {} };

test("uninitialized server shows Setup", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: false });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  expect(await screen.findByText("Create the admin account")).toBeTruthy();
});

test("initialized + unauthenticated shows Login", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
});

test("authenticated shows WorldSelect", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue({ id: "u1" });
  vi.spyOn(entryApi, "listWorlds").mockResolvedValue([]);
  render(Entry, props);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});
