import { describe, it, expect, vi } from "vitest";
import { WsClient } from "./ws-client";
import type { Connect, TransportHandlers } from "./transport";
import type { ServerMsg } from "./wire";

const macrotask = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** A minimal well-formed Welcome frame. */
function welcome(): ServerMsg {
  return {
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
    online_users: [],
    role_capabilities: { all: [], by_type: {} },
  };
}

/** An `error` frame with `code`. */
function errorFrame(code: "world_not_found" | "forbidden" | "internal"): ServerMsg {
  return { type: "error", code, message: "refused" };
}

/** One connection attempt: its callbacks and a way to deliver a frame on it. */
interface Attempt {
  /** The callbacks the client handed to `connect`. */
  handlers: TransportHandlers;
  /** Delivers `msg` to the client on this attempt's connection. */
  push: (msg: ServerMsg) => void;
}

/** A client over a connect that opens at once, recording each attempt. `sleep` resolves
 * immediately when `autoReconnect`, otherwise never, so a reconnect is observable but parked. */
function makeClient(autoReconnect = false) {
  const attempts: Attempt[] = [];
  const connect: Connect = (handlers) => {
    attempts.push({ handlers, push: (m) => handlers.onMessage(JSON.stringify(m)) });
    return Promise.resolve({ send: () => {}, close: () => {} });
  };
  const sleep = vi.fn(() => (autoReconnect ? Promise.resolve() : new Promise<void>(() => {})));
  const onJoinRefused = vi.fn();
  const client = new WsClient({
    world: "w1",
    connect,
    handlers: { onCommand: () => {}, onJoinRefused },
    sleep,
  });
  return { client, attempts, sleep, onJoinRefused };
}

describe("a join refusal before the connection's Welcome", () => {
  it.each(["world_not_found", "forbidden"] as const)(
    "%s stops the client, fires the handler once and schedules no reconnect",
    async (code) => {
      const { client, attempts, sleep, onJoinRefused } = makeClient();
      await client.start();

      attempts[0].push(errorFrame(code));
      attempts[0].handlers.onClose();

      expect(client.running).toBe(false);
      expect(onJoinRefused).toHaveBeenCalledTimes(1);
      expect(onJoinRefused).toHaveBeenCalledWith(code);
      expect(sleep).not.toHaveBeenCalled();
    },
  );

  it("internal is not terminal: the client keeps running and reconnects", async () => {
    const { client, attempts, sleep, onJoinRefused } = makeClient();
    await client.start();

    attempts[0].push(errorFrame("internal"));
    attempts[0].handlers.onClose();

    expect(client.running).toBe(true);
    expect(onJoinRefused).not.toHaveBeenCalled();
    expect(sleep).toHaveBeenCalledTimes(1);
    client.stop();
  });

  it("a forbidden on a superseded connection does nothing", async () => {
    const { client, attempts, onJoinRefused } = makeClient();
    await client.start();
    client.stop();
    await client.start();
    expect(attempts).toHaveLength(2);

    attempts[0].push(errorFrame("forbidden"));

    expect(client.running).toBe(true);
    expect(onJoinRefused).not.toHaveBeenCalled();
    client.stop();
  });

  it("a forbidden on a reconnect after an earlier Welcome is terminal", async () => {
    const { client, attempts, onJoinRefused } = makeClient(true);
    await client.start();
    attempts[0].push(welcome());
    attempts[0].handlers.onClose();
    await vi.waitFor(() => expect(attempts).toHaveLength(2));
    await macrotask();

    attempts[1].push(errorFrame("forbidden"));

    expect(client.running).toBe(false);
    expect(onJoinRefused).toHaveBeenCalledTimes(1);
    expect(onJoinRefused).toHaveBeenCalledWith("forbidden");
  });
});

describe("an error frame after the connection's Welcome", () => {
  it.each(["world_not_found", "forbidden"] as const)("%s is ignored", async (code) => {
    const { client, attempts, onJoinRefused } = makeClient();
    await client.start();
    attempts[0].push(welcome());

    attempts[0].push(errorFrame(code));

    expect(client.running).toBe(true);
    expect(onJoinRefused).not.toHaveBeenCalled();
    client.stop();
  });
});
