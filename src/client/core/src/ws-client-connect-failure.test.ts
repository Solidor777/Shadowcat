// @vitest-environment node
import { describe, it, expect } from "vitest";
import { WsClient } from "./ws-client";
import type { ServerMsg } from "./wire";
import type { Connect } from "./transport";

// `onConnectFailure` reports consecutive failed attempts before the FIRST welcome and goes silent
// for good once one arrives, so a mid-session drop never reads as "cannot connect".

const flush = () => new Promise((r) => setTimeout(r, 0));

/** A backoff sleep that yields to the macrotask queue: an immediately-resolved sleep would spin
 * the retry loop in microtasks and starve the test's own timers. */
const macroSleep = () => new Promise<void>((r) => setTimeout(r, 1));

const WELCOME: ServerMsg = {
  type: "welcome",
  world: "w",
  current_seq: 0,
  server_time: 0,
  server_version: "0.0.0-test",
  world_default_grants: { by_role: {}, by_user: {} },
  user_role: "player",
  capability_requirements: [],
  contract_declarations: [],
  schema_declarations: [],
  role_capabilities: { all: [], by_type: {} },
};

describe("WsClient.onConnectFailure", () => {
  it("counts failed connects up from 1 while never welcomed", async () => {
    const counts: number[] = [];
    const client = new WsClient({
      world: "w",
      connect: () => Promise.reject(new Error("tls refused")),
      handlers: { onCommand: () => {}, onConnectFailure: (n) => counts.push(n) },
      sleep: macroSleep,
    });
    await client.start();
    for (let i = 0; i < 20 && counts.length < 3; i++) await flush();
    client.stop();
    expect(counts.slice(0, 3)).toEqual([1, 2, 3]);
  });

  it("counts a socket that opens but is never welcomed", async () => {
    const counts: number[] = [];
    const connect: Connect = (h) =>
      Promise.resolve({ send: () => {}, close: () => h.onClose() });
    const client = new WsClient({
      world: "w",
      connect,
      handlers: { onCommand: () => {}, onConnectFailure: (n) => counts.push(n) },
      sleep: macroSleep,
      welcomeTimeoutMs: 5,
    });
    await client.start();
    for (let i = 0; i < 100 && counts.length < 2; i++) await new Promise((r) => setTimeout(r, 5));
    client.stop();
    expect(counts.slice(0, 2)).toEqual([1, 2]);
  });

  it("never fires after a welcome: a later drop is not a first-connect failure", async () => {
    const counts: number[] = [];
    let onMessage: (d: string) => void = () => {};
    let onClose: () => void = () => {};
    const client = new WsClient({
      world: "w",
      connect: (h) => {
        onMessage = h.onMessage;
        onClose = h.onClose;
        return Promise.resolve({ send: () => {}, close: () => {} });
      },
      handlers: { onCommand: () => {}, onConnectFailure: (n) => counts.push(n) },
      sleep: () => new Promise<void>(() => {}),
    });
    await client.start();
    onMessage(JSON.stringify(WELCOME));
    onClose();
    await flush();
    client.stop();
    expect(counts).toEqual([]);
  });
});
