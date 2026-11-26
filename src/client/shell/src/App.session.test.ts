// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/svelte";
import { silentLogger } from "@shadowcat/core";
import { test, expect, vi, afterEach } from "vitest";
import { notifications, type AppContext } from "@shadowcat/ui-kit";
import type { TlsStatus } from "@shadowcat/types";
import App from "./App.svelte";
import * as api from "./lib/api";
import AppContextCapture from "./lib/__fixtures__/AppContextCapture.svelte";
import { captured } from "./__fixtures__/entryCapture";
import type { WorldSession } from "./lib/worldSession.svelte";

// App-level wiring of the session's lifetime: a welcome reaches the TLS status loader, and the
// Table App mounts follows its session. The pieces are unit-tested apart (TlsStatusLoader,
// WorldSession.onWelcome, Table's session guard); this file drives them through the real App.

const hoisted = vi.hoisted(() => ({
  sessions: [] as unknown[],
  /** The options each session was constructed with, parallel to `sessions`. */
  options: [] as Array<{
    onEvicted?: () => void;
    onJoinRefused?: (code: "world_not_found" | "forbidden") => void;
  }>,
  eagerRole: false,
}));

vi.mock("@shadowcat/module-entry", async () => {
  const { default: Stub } = await import("./__fixtures__/EntryStub.svelte");
  return { Entry: Stub };
});

// The status loader makes one attempt per `ensure()`: a failed read stays failed until something
// calls `ensure()` again, which is what the welcome wiring is for.
vi.mock("./lib/tlsStatusLoader.svelte", async (importActual) => {
  const actual = await importActual<typeof import("./lib/tlsStatusLoader.svelte")>();
  class SingleAttemptLoader extends actual.TlsStatusLoader {
    constructor(load: () => Promise<TlsStatus>) {
      super(load, [], async () => {});
    }
  }
  return { ...actual, TlsStatusLoader: SingleAttemptLoader };
});

// `App` lists every first-party module; the sessions here run none, so the real Table mounts over
// a bare contribution registry instead of a WebGL stage.
vi.mock("./lib/worldSession.svelte", async (importActual) => {
  const actual = await importActual<typeof import("./lib/worldSession.svelte")>();
  class BareSession extends actual.WorldSession {
    constructor(opts: ConstructorParameters<typeof actual.WorldSession>[0]) {
      super({ ...opts, modules: [], logger: silentLogger });
      // A session whose role is already known when it is created, so `App` renders its Table at
      // once: the only way a Table for one session can be handed a different session.
      if (hoisted.eagerRole) this.role = "player";
      hoisted.sessions.push(this);
      hoisted.options.push(opts);
    }
  }
  return { ...actual, WorldSession: BareSession };
});

vi.mock("@shadowcat/core", async (importActual) => {
  const actual = await importActual<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listWorldMembers: vi.fn().mockResolvedValue([]),
    listInstalledModules: vi.fn().mockResolvedValue([]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
  };
});

const SELF_SIGNED: TlsStatus = { enabled: true, source: "self-signed", https_origins: [], alpn: ["h2", "http/1.1"] };

/** A WebSocket that opens and delivers a `welcome` for the world named in its URL. */
class WelcomingSocket {
  private readonly listeners = new Map<string, Array<(e: unknown) => void>>();
  constructor(url: string) {
    const world = new URL(url, "http://localhost").searchParams.get("world");
    setTimeout(() => {
      this.fire("open", {});
      this.fire("message", {
        data: JSON.stringify({
          type: "welcome",
          world,
          current_seq: 0,
          server_time: 0,
          server_version: "0.1.0",
          world_default_grants: { by_role: {}, by_user: {} },
          user_role: "player",
          capability_requirements: [],
          contract_declarations: [],
          schema_declarations: [],
          online_users: [],
        }),
      });
    }, 0);
  }
  private fire(type: string, event: unknown) {
    for (const fn of this.listeners.get(type) ?? []) fn(event);
  }
  addEventListener(type: string, fn: (e: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  send() {}
  close() {}
}

afterEach(() => {
  cleanup();
  for (const s of hoisted.sessions.splice(0)) (s as WorldSession).leave();
  hoisted.options.splice(0);
  captured.onEnterWorld = undefined;
  hoisted.eagerRole = false;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  location.hash = "";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
});

/** Boots `App` to the entry route (signed in, no saved world) and returns the context-capture
 * hook: contributes a surface to `session` that records the `AppContext` of the Table mounted
 * over it. */
async function bootToEntry(getTlsStatus: () => Promise<TlsStatus>) {
  vi.stubGlobal("WebSocket", WelcomingSocket as unknown);
  vi.spyOn(api, "getTlsStatus").mockImplementation(getTlsStatus);
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ modules: {}, global: { locale: "en", lastWorld: null }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "getWorldSnapshot").mockRejectedValue(new Error("no snapshot in this test"));
  vi.spyOn(api, "listWorlds").mockResolvedValue([]);
  render(App);
  await vi.waitFor(() => expect(captured.onEnterWorld).toBeDefined());
}

/** Enters `world` through the entry screen's callback, waits for the session `App` opens for the
 * resulting route, and records the AppContext of the Table that mounts for it. */
async function enter(world: string): Promise<{ ctx: () => AppContext | undefined }> {
  let ctx: AppContext | undefined;
  const before = hoisted.sessions.length;
  captured.onEnterWorld!(world);
  await vi.waitFor(() => expect(hoisted.sessions.length).toBe(before + 1));
  const session = hoisted.sessions.at(-1) as WorldSession;
  session.contributions.contribute({
    id: `context-capture-${world}`,
    contract: "shadowcat.surface:root",
    component: AppContextCapture,
    props: { onContext: (c: AppContext) => (ctx = c) },
  });
  return { ctx: () => ctx };
}

test("a welcome re-reads a TLS status whose first read failed, and the status reaches the mounted Table", async () => {
  const read = vi.spyOn(api, "getTlsStatus");
  let attempt = 0;
  await bootToEntry(async () => {
    attempt += 1;
    if (attempt === 1) throw new Error("down");
    return SELF_SIGNED;
  });
  const entered = await enter("w1");
  await vi.waitFor(() => expect(entered.ctx()).toBeDefined());
  // Same session throughout: the Table re-renders when `tls` changes and must not throw.
  await vi.waitFor(() => expect(entered.ctx()!.tls).toEqual(SELF_SIGNED));
  expect(read).toHaveBeenCalledTimes(2);
});

test("entering a second world while a Table is mounted gives the new session its own Table without throwing", async () => {
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());
  expect(first.ctx()!.world).toBe("w1");

  const second = await enter("w2");
  await vi.waitFor(() => expect(second.ctx()).toBeDefined());
  expect(second.ctx()!.world).toBe("w2");
  expect(second.ctx()).not.toBe(first.ctx());
});

test("a session swapped in while the Table is mounted is given a fresh Table, not the old one re-bound", async () => {
  hoisted.eagerRole = true;
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());

  // `App` renders the Table as soon as a session has a role and a world, so this swap hands the
  // mounted Table a different session unless `App` remounts it.
  const second = await enter("w2");
  await vi.waitFor(() => expect(second.ctx()).toBeDefined());
  expect(second.ctx()!.world).toBe("w2");
});

/** Points the URL hash at `hash` and delivers the `hashchange` a browser would (jsdom does not
 * dispatch it for a bare assignment). */
function navigateBrowser(hash: string) {
  location.hash = hash;
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}

test("a hashchange to another world's route leaves the old session and enters that world", async () => {
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());
  const firstSession = hoisted.sessions.at(-1) as WorldSession;
  const leave = vi.spyOn(firstSession, "leave");
  const enterSpy = vi.spyOn(Object.getPrototypeOf(Object.getPrototypeOf(firstSession)), "enter");

  navigateBrowser("#/world/w2");
  await vi.waitFor(() => expect(hoisted.sessions.length).toBe(2));
  expect(leave).toHaveBeenCalled();
  expect(enterSpy).toHaveBeenCalledWith("w2");
  const second = hoisted.sessions.at(-1) as WorldSession;
  let ctx: AppContext | undefined;
  second.contributions.contribute({
    id: "context-capture-w2",
    contract: "shadowcat.surface:root",
    component: AppContextCapture,
    props: { onContext: (c: AppContext) => (ctx = c) },
  });
  await vi.waitFor(() => expect(ctx).toBeDefined());
  expect(ctx!.world).toBe("w2");
});

test("a hashchange to another world while the first snapshot is pending leaves exactly one socket, the second world's", async () => {
  const sockets: string[] = [];
  class CountingSocket extends WelcomingSocket {
    constructor(url: string) {
      super(url);
      sockets.push(new URL(url, "http://localhost").searchParams.get("world") ?? "");
    }
  }
  await bootToEntry(async () => SELF_SIGNED);
  vi.stubGlobal("WebSocket", CountingSocket as unknown);
  let releaseFirst!: () => void;
  vi.spyOn(api, "getWorldSnapshot").mockImplementation((world: string) =>
    world === "w1"
      ? new Promise((resolve) => {
          releaseFirst = () => resolve({ documents: [], seq: 0 });
        })
      : Promise.reject(new Error("no snapshot in this test")),
  );

  navigateBrowser("#/world/w1");
  await vi.waitFor(() => expect(hoisted.sessions.length).toBe(1));
  navigateBrowser("#/world/w2");
  await vi.waitFor(() => expect(hoisted.sessions.length).toBe(2));
  await vi.waitFor(() => expect(sockets).toEqual(["w2"]));
  releaseFirst();
  await new Promise((r) => setTimeout(r, 20));
  expect(sockets).toEqual(["w2"]);
});

test("the world socket is opened under the route prefix the server page declared", async () => {
  (globalThis as Record<string, unknown>).__SHADOWCAT_BASE__ = "/ttrpg";
  try {
    const urls: string[] = [];
    class RecordingSocket extends WelcomingSocket {
      constructor(url: string) {
        super(url);
        urls.push(url);
      }
    }
    await bootToEntry(async () => SELF_SIGNED);
    vi.stubGlobal("WebSocket", RecordingSocket as unknown);
    navigateBrowser("#/world/w1");
    await vi.waitFor(() => expect(urls.length).toBeGreaterThan(0));
    expect(new URL(urls[0]).pathname).toBe("/ttrpg/ws");
    expect(new URL(urls[0]).searchParams.get("world")).toBe("w1");
  } finally {
    delete (globalThis as Record<string, unknown>).__SHADOWCAT_BASE__;
  }
});

test("Back to a world route after leaving it enters a new session for that world", async () => {
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());
  const firstSession = hoisted.sessions.at(-1) as WorldSession;
  const leave = vi.spyOn(firstSession, "leave");

  navigateBrowser("#/worlds");
  await vi.waitFor(() => expect(leave).toHaveBeenCalled());
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();

  navigateBrowser("#/world/w1");
  await vi.waitFor(() => expect(hoisted.sessions.length).toBe(2));
  expect(hoisted.sessions.at(-1)).not.toBe(firstSession);
});

test("a world route without an identity shows the entry screen, not a permanent Connecting frame", async () => {
  vi.stubGlobal("WebSocket", WelcomingSocket as unknown);
  vi.spyOn(api, "getTlsStatus").mockResolvedValue(SELF_SIGNED);
  vi.spyOn(api, "getMe").mockResolvedValue(null);
  render(App);
  await vi.waitFor(() => expect(captured.onEnterWorld).toBeDefined());

  navigateBrowser("#/world/w1");
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();
  expect(screen.queryByText("Connecting…")).toBeNull();
  expect(hoisted.sessions).toHaveLength(0);
});

test.each([
  ["world_not_found", "That world no longer exists."],
  ["forbidden", "You are not a member of that world."],
] as const)("a %s join refusal warns the user and returns to the worlds list", async (code, text) => {
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());
  const session = hoisted.sessions.at(-1) as WorldSession;
  const leave = vi.spyOn(session, "leave");
  const push = vi.spyOn(notifications, "push");

  hoisted.options.at(-1)!.onJoinRefused!(code);

  expect(push).toHaveBeenCalledWith("warning", text);
  await vi.waitFor(() => expect(leave).toHaveBeenCalled());
  expect(location.hash).toBe("#/worlds");
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();
});

test("an eviction returns to the worlds list and leaves the session", async () => {
  await bootToEntry(async () => SELF_SIGNED);
  const first = await enter("w1");
  await vi.waitFor(() => expect(first.ctx()).toBeDefined());
  const session = hoisted.sessions.at(-1) as WorldSession;
  const leave = vi.spyOn(session, "leave");

  hoisted.options.at(-1)!.onEvicted!();
  await vi.waitFor(() => expect(leave).toHaveBeenCalled());
  expect(location.hash).toBe("#/worlds");
  expect(await screen.findByTestId("entry-stub")).toBeTruthy();
});
