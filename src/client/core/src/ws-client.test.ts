import { describe, it, expect } from "vitest";
import { WsClient } from "./ws-client";
import { MockServer } from "./mock-server";
import { DocumentStore } from "./store";
import type { ClientMsg, ServerMsg, WireOperation } from "./wire";
import type { Connect } from "./transport";

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
      parent_id: null,
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

  it("dispatches asset_changed frames to onAssetChanged", async () => {
    const seen: Array<{ uuid: string; op: string }> = [];
    // A bare transport whose handlers we capture, so the test can push an
    // arbitrary out-of-band server frame.
    let push!: (frame: ServerMsg) => void;
    const connect: Connect = (handlers) => {
      push = (frame) => handlers.onMessage(JSON.stringify(frame));
      return Promise.resolve({ send: () => {}, close: () => {} });
    };
    const client = new WsClient({
      connect,
      handlers: { onCommand: () => {}, onAssetChanged: (m) => seen.push(m) },
    });
    await client.start();
    push({ type: "asset_changed", uuid: "a1", op: "replaced" });
    await flush();
    expect(seen).toEqual([{ uuid: "a1", op: "replaced" }]);
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

  it("subscribeSearch fires onUpdate for initial + updates, and unsubscribe stops dispatch", async () => {
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
    let calls = 0;
    const p = client.subscribeSearch("dragon", { limit: 5 }, () => {
      calls += 1;
    });
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
    expect(req.subscribe).toBe(true);
    onMessage(
      JSON.stringify({ type: "search_result", request_id: req.request_id, hits: [], next_cursor: null }),
    );
    const handle = await p;
    expect(calls).toBe(1); // initial fired via onUpdate

    onMessage(JSON.stringify({ type: "search_update", request_id: req.request_id, hits: [] }));
    expect(calls).toBe(2);

    handle.unsubscribe();
    expect(sent.some((s) => JSON.parse(s).type === "unsubscribe")).toBe(true);
    onMessage(JSON.stringify({ type: "search_update", request_id: req.request_id, hits: [] }));
    expect(calls).toBe(2); // no further dispatch after unsubscribe
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

  it("search rejects immediately when there is no live transport", async () => {
    const client = new WsClient({
      connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
      handlers: noop,
    });
    // Not started → transport is null. A long timeout would otherwise hang.
    await expect(
      client.search("x", { timeoutMs: 60_000 }),
    ).rejects.toThrow(/not connected/i);
  });

  it("subscribeSearch rejects immediately when there is no live transport", async () => {
    const client = new WsClient({
      connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
      handlers: noop,
    });
    await expect(
      client.subscribeSearch("x", { timeoutMs: 60_000 }, () => {}),
    ).rejects.toThrow(/not connected/i);
  });

  it("a throwing onUpdate is surfaced, not thrown into the socket loop", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const errors: unknown[] = [];
    const client = new WsClient({
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: { onCommand: () => {}, onError: (e) => errors.push(e) },
    });
    await client.start();
    let calls = 0;
    const p = client.subscribeSearch("dragon", { limit: 5 }, () => {
      calls += 1;
      if (calls === 1) throw new Error("handler boom");
    });
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "search")!);
    // Initial result fires onUpdate, which throws. The throw must not prevent the
    // subscription promise from resolving, and must be routed to onError.
    onMessage(
      JSON.stringify({ type: "search_result", request_id: req.request_id, hits: [], next_cursor: null }),
    );
    const handle = await p;
    expect(calls).toBe(1);
    expect(errors).toHaveLength(1);
    // The subscription is still live: a later update still dispatches.
    onMessage(JSON.stringify({ type: "search_update", request_id: req.request_id, hits: [] }));
    expect(calls).toBe(2);
    handle.unsubscribe();
  });

  it("subscribeScene fires onUpdate on each scene_derived; unsubscribe stops dispatch", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      connect: (h) => { onMessage = h.onMessage; return Promise.resolve({ send: (d) => sent.push(d), close: () => {} }); },
      handlers: noop,
    });
    await client.start();
    const frames: Array<{ payload: unknown; computedAtSeq: number }> = [];
    const p = client.subscribeScene("identity", (f) => frames.push(f));
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "scene_subscribe")!);
    expect(req.channel).toBe("identity");
    onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 3, payload: { entity_count: 0 } }));
    const handle = await p;
    expect(frames).toEqual([{ payload: { entity_count: 0 }, computedAtSeq: 3 }]);
    onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 4, payload: { entity_count: 2 } }));
    expect(frames).toHaveLength(2);
    handle.unsubscribe();
    expect(sent.some((s) => JSON.parse(s).type === "scene_unsubscribe")).toBe(true);
    onMessage(JSON.stringify({ type: "scene_derived", request_id: req.request_id, channel: "identity", computed_at_seq: 5, payload: {} }));
    expect(frames).toHaveLength(2); // no dispatch after unsubscribe
  });

  it("subscribeScene rejects on a scene_error frame", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      connect: (h) => { onMessage = h.onMessage; return Promise.resolve({ send: (d) => sent.push(d), close: () => {} }); },
      handlers: noop,
    });
    await client.start();
    const p = client.subscribeScene("nope", () => {});
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "scene_subscribe")!);
    onMessage(JSON.stringify({ type: "scene_error", request_id: req.request_id, message: "unknown channel" }));
    await expect(p).rejects.toThrow(/unknown channel/);
  });

  it("subscribeScene rejects immediately with no live transport", async () => {
    const client = new WsClient({ connect: () => Promise.resolve({ send: () => {}, close: () => {} }), handlers: noop });
    await expect(client.subscribeScene("identity", () => {}, { timeoutMs: 60_000 })).rejects.toThrow(/not connected/i);
  });

  it("pathfind resolves on path_result and rejects on path_error", async () => {
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

    const p = client.pathfind("scene-1", [50, 50], [[250, 50]], 0.5);
    const sentFrame = JSON.parse(sent.find((s) => JSON.parse(s).type === "pathfind")!);
    expect(sentFrame.type).toBe("pathfind");
    onMessage(
      JSON.stringify({
        type: "path_result",
        request_id: sentFrame.request_id,
        path: [[50, 50], [250, 50]],
        cost: 2,
      }),
    );
    await expect(p).resolves.toEqual({ path: [[50, 50], [250, 50]], cost: 2 });

    const p2 = client.pathfind("scene-1", [50, 50], [[9999, 9999]], 0.5);
    const sentFrame2 = JSON.parse(sent.find((s, i) => i > sent.indexOf(JSON.stringify(sentFrame)) && JSON.parse(s).type === "pathfind")!);
    onMessage(
      JSON.stringify({
        type: "path_error",
        request_id: sentFrame2.request_id,
        message: "unreachable",
      }),
    );
    await expect(p2).rejects.toThrow("unreachable");
  });

  it("moveRequest resolves on move_executed", async () => {
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

    const p = client.moveRequest("scene1", "tok1", [[0, 0], [100, 0]]);
    const sentFrame = JSON.parse(sent.find((s) => JSON.parse(s).type === "move_request")!);
    expect(sentFrame.type).toBe("move_request");
    expect(sentFrame.token_id).toBe("tok1");
    onMessage(
      JSON.stringify({
        type: "move_executed",
        request_id: sentFrame.request_id,
        token_id: "tok1",
        stop: [100, 0],
        render_path: [[0, 0], [100, 0]],
        duration_ms: 200,
      }),
    );
    await expect(p).resolves.toEqual({
      tokenId: "tok1",
      stop: [100, 0],
      renderPath: [[0, 0], [100, 0]],
      durationMs: 200,
    });
  });

  it("moveRequest rejects on move_error", async () => {
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

    const p = client.moveRequest("scene1", "tok1", [[0, 0], [100, 0]]);
    const sentFrame = JSON.parse(sent.find((s) => JSON.parse(s).type === "move_request")!);
    onMessage(
      JSON.stringify({
        type: "move_error",
        request_id: sentFrame.request_id,
        message: "move rejected",
      }),
    );
    await expect(p).rejects.toThrow();
  });
});
