import { describe, it, expect } from "vitest";
import { WsClient } from "./ws-client";
import { MockServer } from "./mock-server";
import { DocumentStore } from "./store";
import type { ClientMsg, WireOperation } from "./wire";

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const noop = { onCommand: () => {} };

async function waitFor(pred: () => boolean, tries = 100): Promise<void> {
  for (let i = 0; i < tries; i++) {
    if (pred()) return;
    await flush();
  }
  throw new Error("waitFor timed out");
}

function createOp(id: string): WireOperation {
  return {
    op: "create",
    doc: {
      id,
      scope: { kind: "world", world_id: "test-world" },
      doc_type: "actor",
      schema_version: 1,
      source: null,
      owner: null,
      permissions: {
        default: "none",
        users: {},
        property_overrides: {},
        capabilities: { by_role: {}, by_user: {} },
      },
      embedded: {},
      system: {},
      created_at: 0,
      updated_at: 0,
    },
  };
}

function intent(n: number, ops: WireOperation[]): ClientMsg {
  return { type: "intent", intent_id: `i${n}`, ops };
}

describe("WsClient", () => {
  it("syncs existing events on join", async () => {
    const server = new MockServer();
    const pub = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
    });
    await pub.start();
    await flush();
    pub.send(intent(1, [createOp("d1")]));
    pub.send(intent(2, [createOp("d2")]));
    await flush();

    const store = new DocumentStore();
    const sub = new WsClient({
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
    });
    await sub.start();
    await waitFor(() => sub.appliedSeq === 2);

    expect(store.get("d1")).toBeDefined();
    expect(store.get("d2")).toBeDefined();
  });

  it("applies live events in order", async () => {
    const server = new MockServer();
    const store = new DocumentStore();
    const sub = new WsClient({
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
    });
    await sub.start();
    await flush();

    const pub = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
    });
    await pub.start();
    pub.send(intent(1, [createOp("d1")]));
    await waitFor(() => sub.appliedSeq === 1);

    expect(store.get("d1")).toBeDefined();
  });

  it("drops duplicate replayed events", async () => {
    const server = new MockServer();
    let applied = 0;
    const store = new DocumentStore();
    const sub = new WsClient({
      connect: server.connector("u2"),
      handlers: {
        onCommand: (c) => {
          applied++;
          store.applyCommand(c);
        },
      },
    });
    await sub.start();
    await flush();
    const pub = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
    });
    await pub.start();
    await flush();
    pub.send(intent(1, [createOp("d1")]));
    await waitFor(() => applied === 1);

    // Re-request a range we already have; the sequence guard drops it.
    sub.send({ type: "resync_request", from_seq: 1 });
    await flush();
    await flush();
    expect(applied).toBe(1);
    expect(sub.appliedSeq).toBe(1);
  });

  it("reconnects and catches up after a dropped connection", async () => {
    const server = new MockServer();
    const store = new DocumentStore();
    const sub = new WsClient({
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
      sleep: () => Promise.resolve(),
    });
    await sub.start();
    await flush();
    const pub = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
    });
    await pub.start();
    await flush();
    pub.send(intent(1, [createOp("d1")]));
    await waitFor(() => sub.appliedSeq === 1);

    server.drop("u2");
    // Publish while the subscriber is down; it catches up via resync on
    // reconnect (immediate backoff) regardless of timing.
    pub.send(intent(2, [createOp("d2")]));
    await waitFor(() => sub.appliedSeq === 2);
    expect(store.get("d2")).toBeDefined();
  });

  it("a throwing onCommand is surfaced, not thrown into the socket loop", async () => {
    const server = new MockServer();
    const errors: unknown[] = [];
    let seen = 0;
    const sub = new WsClient({
      connect: server.connector("u2"),
      handlers: {
        onCommand: () => {
          seen++;
          if (seen === 1) throw new Error("apply failed");
        },
        onError: (e) => errors.push(e),
      },
    });
    await sub.start();
    await flush();
    const pub = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
    });
    await pub.start();
    pub.send(intent(1, [createOp("d1")]));
    pub.send(intent(2, [createOp("d2")]));
    // First apply throws → surfaced via onError; loop survives and advances,
    // so the second command is still delivered.
    await waitFor(() => sub.appliedSeq === 2);
    expect(errors).toHaveLength(1);
    expect(seen).toBe(2);
  });

  it("computes the server time offset from Welcome", async () => {
    const server = new MockServer({ now: () => 5000 });
    const c = new WsClient({
      connect: server.connector("u1"),
      handlers: noop,
      now: () => 1000,
    });
    await c.start();
    await waitFor(() => c.serverNow() === 5000);
    expect(c.serverNow()).toBe(5000);
  });

  it("search resolves with the correlated result", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();
    const p = client.search("dragon", { limit: 5 });
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
    expect(req.query).toBe("dragon");
    onMessage(
      JSON.stringify({
        type: "search_result",
        request_id: req.request_id,
        hits: [],
        next_cursor: "7",
      }),
    );
    await expect(p).resolves.toEqual({ hits: [], nextCursor: "7" });
  });

  it("search rejects on a search_error frame", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();
    const p = client.search("x");
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
    onMessage(
      JSON.stringify({
        type: "search_error",
        request_id: req.request_id,
        message: "boom",
      }),
    );
    await expect(p).rejects.toThrow("boom");
  });

  it("search rejects on timeout", async () => {
    const client = new WsClient({
      connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
      handlers: noop,
    });
    await client.start();
    await expect(client.search("x", { timeoutMs: 1 })).rejects.toThrow(/timeout/i);
  });

  it("stop() rejects in-flight searches instead of leaving them hanging", async () => {
    const client = new WsClient({
      connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
      handlers: noop,
    });
    await client.start();
    const p = client.search("x", { timeoutMs: 60_000 });
    client.stop();
    await expect(p).rejects.toThrow(/stopped/i);
  });
});
