// @vitest-environment jsdom
// @vitest-environment-options {"url": "https://192.0.2.10:30000/"}
import { render } from "@testing-library/svelte";
import { test, expect, vi, afterEach } from "vitest";
import { notifications } from "@shadowcat/ui-kit";
import type { TlsStatus } from "@shadowcat/types";
import App from "./App.svelte";
import * as api from "./lib/api";
import { WS_HINT_AFTER_FAILURES } from "@shadowcat/core";
import { plainHttpHint } from "./lib/plainHttpHint";
import { TLS_RETRY_DELAYS_MS } from "./lib/tlsStatusLoader.svelte";
import { WorldSession } from "./lib/worldSession.svelte";

// The wiring from a run of failed first WebSocket connects on an https page, through App, to the
// plain-HTTP hint notification. `plainHttpHint`'s own decision table is in plainHttpHint.test.ts;
// this file proves App passes it the real probe, the status loader and the page location.

vi.mock("@shadowcat/module-entry", async () => {
  const { default: Stub } = await import("./__fixtures__/EntryStub.svelte");
  return { Entry: Stub };
});

// The status loader keeps its real retry count but sleeps for no time, so "every attempt failed"
// is reached in microtasks rather than after the production backoff (about a minute).
vi.mock("./lib/tlsStatusLoader.svelte", async (importActual) => {
  const actual = await importActual<typeof import("./lib/tlsStatusLoader.svelte")>();
  class NoBackoffLoader extends actual.TlsStatusLoader {
    constructor(load: () => Promise<TlsStatus>) {
      super(load, actual.TLS_RETRY_DELAYS_MS, async () => {});
    }
  }
  return { ...actual, TlsStatusLoader: NoBackoffLoader };
});

// Wraps the real decision so a test can await the one `App` made instead of sleeping.
vi.mock("./lib/plainHttpHint", async (importActual) => {
  const actual = await importActual<typeof import("./lib/plainHttpHint")>();
  return { ...actual, plainHttpHint: vi.fn(actual.plainHttpHint) };
});

const SELF_SIGNED: TlsStatus = { enabled: true, source: "self-signed", https_origins: [], alpn: ["h2", "http/1.1"] };

/** Bumped after every test so a session a previous test left running stops receiving events (an
 * unmounted `App` does not tear its session down). */
let generation = 0;

/** A WebSocket whose every connection attempt fails at once, like a browser refusing `wss:`. */
class RefusedSocket {
  private readonly listeners = new Map<string, Array<(e: unknown) => void>>();
  constructor() {
    const mine = generation;
    setTimeout(() => {
      if (mine !== generation) return;
      for (const type of ["error", "close"]) for (const fn of this.listeners.get(type) ?? []) fn({ code: 1006 });
    }, 0);
  }
  addEventListener(type: string, fn: (e: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), fn]);
  }
  send() {}
  close() {}
}

/** Sessions the rendered `App` opened; an unmounted `App` leaves them reconnecting, so each test
 * leaves them itself. */
const opened: WorldSession[] = [];

afterEach(() => {
  generation += 1;
  for (const session of opened.splice(0)) session.leave();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  location.hash = "";
});

function bootIntoWorld(tls: () => Promise<TlsStatus>, me: () => Promise<{ ok: boolean }>) {
  vi.mocked(plainHttpHint).mockClear();
  vi.stubGlobal("WebSocket", RefusedSocket as unknown);
  vi.stubGlobal("fetch", vi.fn(async () => me()));
  vi.spyOn(api, "getTlsStatus").mockImplementation(tls);
  vi.spyOn(api, "getMe").mockResolvedValue({ id: "u1", username: "gm", server_role: "user" });
  vi.spyOn(api, "getUiState").mockResolvedValue({ global: { locale: "en", lastWorld: "w1" }, worlds: {} });
  vi.spyOn(api, "putUiState").mockResolvedValue();
  vi.spyOn(api, "listWorlds").mockResolvedValue([{ id: "w1", name: "W", role: "gm" }]);
  const realEnter = WorldSession.prototype.enter;
  vi.spyOn(WorldSession.prototype, "enter").mockImplementation(function (this: WorldSession, ...args) {
    opened.push(this);
    return realEnter.apply(this, args);
  });
  const push = vi.spyOn(notifications, "push");
  render(App);
  return push;
}

test("three refused WebSocket connects with a working probe offer the plain address", async () => {
  const push = bootIntoWorld(async () => SELF_SIGNED, async () => ({ ok: true }));
  await vi.waitFor(() => expect(push).toHaveBeenCalledWith("warning", expect.stringContaining("http://192.0.2.10:30000/")), {
    timeout: 8000,
  });
});

test("a probe answered 401 (a bad session) offers nothing", async () => {
  const push = bootIntoWorld(async () => SELF_SIGNED, async () => ({ ok: false }));
  const fetchMock = globalThis.fetch as unknown as ReturnType<typeof vi.fn>;
  await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled(), { timeout: 8000 });
  await new Promise((r) => setTimeout(r, 200));
  expect(push).not.toHaveBeenCalled();
});

test("an unreadable TLS status offers nothing", async () => {
  const read = vi.fn(async (): Promise<TlsStatus> => Promise.reject(new Error("down")));
  const push = bootIntoWorld(read, async () => ({ ok: true }));
  // The hint decision is made on the failure count the hint fires at; its result is the
  // observation. Earlier failure counts return null without looking at the status.
  const decisions = () =>
    vi.mocked(plainHttpHint).mock.calls.flatMap(([n], i) => (n === WS_HINT_AFTER_FAILURES ? [i] : []));
  await vi.waitFor(() => expect(decisions()).not.toHaveLength(0), { timeout: 8000 });
  const at = decisions()[0]!;
  expect(await vi.mocked(plainHttpHint).mock.results[at]!.value).toBeNull();
  // Every attempt of a full retry run failed before the decision was made.
  expect(read.mock.calls.length).toBeGreaterThanOrEqual(actualAttempts());
  expect(push).not.toHaveBeenCalledWith("warning", expect.stringContaining("http://"));
});

/** Attempts in one full loader run: the first read plus one per retry delay. */
function actualAttempts(): number {
  return TLS_RETRY_DELAYS_MS.length + 1;
}
