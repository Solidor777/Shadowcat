import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import type { TlsStatus } from "@shadowcat/types";
import Entry from "./Entry.svelte";
import * as entryApi from "./entryApi";

afterEach(() => vi.restoreAllMocks());

const TLS: TlsStatus = {
  enabled: true,
  source: "self-signed",
  https_origins: [],
  alpn: ["h2", "http/1.1"],
};

const props = { onAuthenticated: () => true, onEnterWorld: () => {} };

async function submitLogin() {
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
  await fireEvent.input(screen.getByLabelText("Username"), { target: { value: "a" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "b" } });
  await fireEvent.click(screen.getByRole("button", { name: "Log in" }));
}

/** Submits the Setup form against a stubbed `setup` that succeeds. */
async function submitSetup() {
  vi.spyOn(entryApi, "setup").mockResolvedValue({ ok: true, status: 200 });
  await fireEvent.input(await screen.findByLabelText("Username"), { target: { value: "a" } });
  await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "b" } });
  await fireEvent.click(screen.getByRole("button", { name: "Create admin" }));
}

test("a finished Setup shows the welcome screen, and Continue leads to Login", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: false, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  await submitSetup();
  expect(await screen.findByText("Your server is ready")).toBeTruthy();
  const guide = screen.getByRole("link", { name: "Read the hosting guide" });
  expect(guide.getAttribute("href")).toContain("hosting.md");
  await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
});

test("a finished Setup goes straight to Login when the server switched tours off", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: false, tls: TLS, onboardingEnabled: false });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  await submitSetup();
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
  expect(screen.queryByText("Your server is ready")).toBeNull();
});

test("an initialized server never shows the welcome screen", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
  expect(screen.queryByText("Your server is ready")).toBeNull();
});

test("uninitialized server shows Setup", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: false, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  expect(await screen.findByText("Create the admin account")).toBeTruthy();
});

test("initialized + unauthenticated shows Login", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  render(Entry, props);
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
});

test("authenticated shows WorldSelect", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue({ id: "u1" });
  vi.spyOn(entryApi, "listWorlds").mockResolvedValue([]);
  render(Entry, props);
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});

test("advances to world-select when onAuthenticated reports success", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null); // unauth → starts at login
  vi.spyOn(entryApi, "login").mockResolvedValue(true);
  vi.spyOn(entryApi, "listWorlds").mockResolvedValue([]);
  render(Entry, { onAuthenticated: () => true, onEnterWorld: () => {} });
  await submitLogin();
  expect(await screen.findByText("Your worlds")).toBeTruthy();
});

test("returns to login when the post-login identity fetch fails", async () => {
  vi.spyOn(entryApi, "getConfig").mockResolvedValue({ initialized: true, tls: TLS, onboardingEnabled: true });
  vi.spyOn(entryApi, "getMe").mockResolvedValue(null);
  vi.spyOn(entryApi, "login").mockResolvedValue(true); // login POST succeeds...
  render(Entry, { onAuthenticated: () => false, onEnterWorld: () => {} }); // ...but identity fetch failed
  await submitLogin();
  // No recovery-into-worlds: the user is returned to login, never stranded on world-select.
  await waitFor(() => expect(screen.getByRole("button", { name: "Log in" })).toBeTruthy());
  expect(screen.queryByText("Your worlds")).toBeNull();
});
