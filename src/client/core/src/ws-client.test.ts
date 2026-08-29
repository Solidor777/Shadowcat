import { describe, it, expect, vi } from "vitest";
import { WsClient } from "./ws-client";
import type { MoveStream } from "./ws-client";
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
      name: null,
      source: null,
      owner: null,
      permissions: {
        default: "none",
        users: {},
        property_overrides: {},
        capabilities: { by_role: {}, by_user: {} },
        gm_role: null,
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

/** A minimal well-formed Welcome frame for tests that only need it to parse. */
function welcomeFrame(): ServerMsg {
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
  };
}

describe("WsClient", () => {
  it("syncs existing events on join", async () => {
    const server = new MockServer();
    const pub = new WsClient({
      world: "w1",
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
      world: "w1",
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
    });
    await sub.start();
    await waitFor(() => sub.appliedSeq === 2);

    expect(store.get("d1")).toBeDefined();
    expect(store.get("d2")).toBeDefined();
  });

  it("dispatches asset_changed frames to onAssetChanged", async () => {
    const seen: Array<{ uuid: string; op: string; version: number }> = [];
    // A bare transport whose handlers we capture, so the test can push an
    // arbitrary out-of-band server frame.
    let push!: (frame: ServerMsg) => void;
    const connect: Connect = (handlers) => {
      push = (frame) => handlers.onMessage(JSON.stringify(frame));
      return Promise.resolve({ send: () => {}, close: () => {} });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: { onCommand: () => {}, onAssetChanged: (m) => seen.push(m) },
    });
    await client.start();
    push({ type: "asset_changed", uuid: "a1", op: "replaced", version: 3 });
    await flush();
    expect(seen).toEqual([{ uuid: "a1", op: "replaced", version: 3 }]);
  });

  it("evicted frame stops the client (no reconnect) and fires onEvicted", async () => {
    let push!: (frame: ServerMsg) => void;
    let opens = 0;
    const connect: Connect = (handlers) => {
      opens += 1;
      push = (frame) => handlers.onMessage(JSON.stringify(frame));
      return Promise.resolve({ send: () => {}, close: () => {} });
    };
    let evictions = 0;
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: { onCommand: () => {}, onEvicted: () => evictions++ },
    });
    await client.start();
    push({ type: "evicted", user: null });
    await flush();
    expect(evictions).toBe(1);
    expect(client.running).toBe(false);
    await flush();
    expect(opens).toBe(1); // no reconnect attempt followed
  });

  it("applies live events in order", async () => {
    const server = new MockServer();
    const store = new DocumentStore();
    const sub = new WsClient({
      world: "w1",
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
    });
    await sub.start();
    await flush();

    const pub = new WsClient({
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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

  it("stop() during a pending connect discards the resolved transport instead of adopting it", async () => {
    let resolveConnect!: (t: { send: () => void; close: () => void }) => void;
    let onCloseHandler!: () => void;
    let closed = false;
    const connect: Connect = (handlers) => {
      onCloseHandler = handlers.onClose;
      return new Promise((resolve) => {
        resolveConnect = resolve;
      });
    };
    const delays: number[] = [];
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: (ms) => {
        delays.push(ms);
        return Promise.resolve();
      },
    });
    const startPromise = client.start();
    client.stop();
    resolveConnect({
      send: () => {},
      close: () => {
        closed = true;
        onCloseHandler();
      },
    });
    await startPromise;
    expect(closed).toBe(true);
    expect(client.connected).toBe(false);
    // close() invoked the real onClose handler, running handleClose() end-to-end — but running_
    // is false, so handleClose()'s own `if (this.running_) this.scheduleReconnect()` guard must
    // skip scheduling a reconnect. An empty `delays` array proves that guard held, not just that
    // the two flags above happened to be set correctly.
    expect(delays).toHaveLength(0);
  });

  it("reconnects and catches up after a dropped connection", async () => {
    const server = new MockServer();
    const store = new DocumentStore();
    const sub = new WsClient({
      world: "w1",
      connect: server.connector("u2"),
      handlers: { onCommand: (c) => store.applyCommand(c) },
      sleep: () => Promise.resolve(),
    });
    await sub.start();
    await flush();
    const pub = new WsClient({
      world: "w1",
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

  it("a connection that never receives Welcome is closed and reconnected after welcomeTimeoutMs", async () => {
    let opens = 0;
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      return Promise.resolve({
        send: () => {},
        // A real Transport's close() fires onClose (see `webSocketConnect`); the mock
        // mirrors that so the watchdog's close() actually engages reconnect.
        close: () => {
          closes.push(id);
          h.onClose();
        },
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    await waitFor(() => opens === 2);
    expect(closes).toEqual([1]);
    // Without this, connection #2 (never welcomed) keeps watchdog-closing and
    // reconnecting unbounded in the vitest worker after the test completes.
    client.stop();
  });

  it("a Welcome inside the window disarms the watchdog (no spurious close)", async () => {
    let opens = 0;
    let onMessage: (d: string) => void = () => {};
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      onMessage = h.onMessage;
      return Promise.resolve({
        send: () => {},
        close: () => {
          closes.push(id);
          h.onClose();
        },
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    onMessage(JSON.stringify(welcomeFrame()));
    // Give the (now-disarmed) watchdog window time to elapse.
    await new Promise((r) => setTimeout(r, 30));
    expect(closes).toEqual([]);
    expect(opens).toBe(1);
  });

  it("the watchdog re-arms on reconnect and a stale timer from a superseded transport no-ops", async () => {
    let opens = 0;
    let onMessage: (d: string) => void = () => {};
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      onMessage = h.onMessage;
      return Promise.resolve({
        send: () => {},
        close: () => {
          closes.push(id);
          h.onClose();
        },
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    // First connection times out and reconnects.
    await waitFor(() => opens === 2);
    // Second connection welcomes immediately, before its own watchdog fires.
    onMessage(JSON.stringify(welcomeFrame()));
    // Advance well past both connections' windows.
    await new Promise((r) => setTimeout(r, 30));
    expect(closes).toEqual([1]); // only the first (superseded) transport ever closed
    expect(opens).toBe(2); // no further reconnect triggered by the stale timer
  });

  it("a stale Welcome delivered after reconnect does not disarm the successor's watchdog", async () => {
    // Pins the interleaving: connection N's Welcome is already queued as a
    // message task when the watchdog closes N; the client reconnects to
    // N+1; the stale Welcome then delivers. It must not disarm N+1's
    // watchdog — if N+1's own Welcome never arrives, N+1 must still time out
    // and reconnect (else the silent hang this task exists to close would be
    // reintroduced).
    let opens = 0;
    const onMessageByGen: Record<number, (d: string) => void> = {};
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      onMessageByGen[id] = h.onMessage;
      return Promise.resolve({
        send: () => {},
        close: () => {
          closes.push(id);
          h.onClose();
        },
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    // Connection 1 times out and reconnects to connection 2.
    await waitFor(() => opens === 2);
    // Connection 1's Welcome arrives late (it was in flight before its close).
    onMessageByGen[1](JSON.stringify(welcomeFrame()));
    // If the stale Welcome incorrectly disarmed connection 2's watchdog,
    // connection 2 would never time out and `opens` would stay at 2.
    await waitFor(() => opens === 3);
    expect(closes).toEqual([1, 2]);
    client.stop();
  });

  it("an opens-but-never-welcomes server backs off with growing reconnect delays, not a fixed watchdog-only cadence", async () => {
    const delays: number[] = [];
    let opens = 0;
    const connect: Connect = (h) => {
      opens += 1;
      return Promise.resolve({
        send: () => {},
        close: () => h.onClose(),
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: (ms) => {
        delays.push(ms);
        return Promise.resolve();
      },
      welcomeTimeoutMs: 5,
      backoffBaseMs: 10,
      backoffMaxMs: 40,
    });
    // Full jitter picks a delay in [ceiling/2, ceiling); pin it to the ceiling
    // itself so the growth assertion below is deterministic.
    const randomSpy = vi.spyOn(Math, "random").mockReturnValue(1);
    try {
      await client.start();
      await waitFor(() => opens >= 4);
    } finally {
      client.stop();
      randomSpy.mockRestore();
    }
    // Each watchdog-close reconnects at the CURRENT attempt's backoff
    // (10 -> 20 -> 40, capped at backoffMaxMs), never resetting to the base
    // delay merely because the socket opened — only a Welcome resets it.
    expect(delays.slice(0, 3)).toEqual([10, 20, 40]);
  });

  it("a welcomed connection resets the reconnect attempt counter — a later drop backs off from the base delay again", async () => {
    const delays: number[] = [];
    let opens = 0;
    const closers: Array<() => void> = [];
    const connect: Connect = (h) => {
      opens += 1;
      const close = () => h.onClose();
      closers.push(close);
      if (opens === 2) {
        // Deliver connection 2's Welcome SYNCHRONOUSLY, inside `connect`,
        // before this call even returns its promise — deterministically
        // welcomed before its own watchdog can possibly arm (armWelcomeWatchdog
        // runs only after `connect`'s promise resolves). Avoids racing a real
        // watchdog timer with a real-time `onMessage` delivery (with real
        // timers, a 5ms-class window leaves ~0ms of margin for the latter).
        h.onMessage(JSON.stringify(welcomeFrame()));
      }
      return Promise.resolve({ send: () => {}, close });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: (ms) => {
        delays.push(ms);
        return Promise.resolve();
      },
      welcomeTimeoutMs: 5,
      backoffBaseMs: 10,
      backoffMaxMs: 40,
    });
    const randomSpy = vi.spyOn(Math, "random").mockReturnValue(1);
    try {
      await client.start();
      // Connection 1 never welcomes; the watchdog closes it -> one reconnect
      // at the base delay (attempt 0). Connection 2 is welcomed synchronously
      // (above) as soon as it opens, resetting the attempt counter.
      await waitFor(() => opens === 2);
      expect(delays).toEqual([10]);
      // A later drop of the now-WELCOMED connection (not a watchdog close)
      // must back off from the base delay again, not continue growing from a
      // stale attempt count carried over from before the Welcome.
      closers[1]();
      await waitFor(() => opens === 3);
      expect(delays).toEqual([10, 10]);
    } finally {
      client.stop();
      randomSpy.mockRestore();
    }
  });

  it("a Connect delivering Welcome before its own continuation runs does not leave a permanently-doomed watchdog", async () => {
    // Pins the out-of-order-delivery contract structurally: if a Welcome is
    // processed before armWelcomeWatchdog's own call site runs (a Connect
    // that resolves handlers.onMessage in a microtask ahead of resolving its
    // returned promise), the watchdog must not arm at all — arming would
    // create a timer nothing will ever disarm, closing a healthy connection.
    let opens = 0;
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      return new Promise((resolve) => {
        queueMicrotask(() => h.onMessage(JSON.stringify(welcomeFrame())));
        queueMicrotask(() =>
          resolve({
            send: () => {},
            close: () => {
              closes.push(id);
              h.onClose();
            },
          }),
        );
      });
    };
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: noop,
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    // Give a (wrongly) armed watchdog's window plenty of time to elapse.
    await new Promise((r) => setTimeout(r, 30));
    expect(closes).toEqual([]);
    client.stop();
  });

  it("a stale resync_end from a superseded connection does not fire onResyncComplete on the successor", async () => {
    // Mirrors the "stale Welcome" test above but for resync_end: connection
    // 1's queued resync_end must not fire the callback once the client has
    // moved on to connection 2.
    let opens = 0;
    const onMessageByGen: Record<number, (d: string) => void> = {};
    const closes: number[] = [];
    const connect: Connect = (h) => {
      const id = ++opens;
      onMessageByGen[id] = h.onMessage;
      return Promise.resolve({
        send: () => {},
        close: () => {
          closes.push(id);
          h.onClose();
        },
      });
    };
    let resyncCompletes = 0;
    const client = new WsClient({
      world: "w1",
      connect,
      handlers: { onCommand: () => {}, onResyncComplete: () => resyncCompletes++ },
      sleep: () => Promise.resolve(),
      welcomeTimeoutMs: 5,
    });
    await client.start();
    // Connection 1 times out and reconnects to connection 2.
    await waitFor(() => opens === 2);
    expect(resyncCompletes).toBe(0);
    // Connection 1's resync_end arrives late (queued before its watchdog close).
    onMessageByGen[1](JSON.stringify({ type: "resync_end", current_seq: 5 }));
    await flush();
    expect(resyncCompletes).toBe(0);
    client.stop();
  });

  it("a throwing onCommand is surfaced, not thrown into the socket loop", async () => {
    const server = new MockServer();
    const errors: unknown[] = [];
    let seen = 0;
    const sub = new WsClient({
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
      world: "w1",
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
    const client = new WsClient({ world: "w1", connect: () => Promise.resolve({ send: () => {}, close: () => {} }), handlers: noop });
    await expect(client.subscribeScene("identity", () => {}, { timeoutMs: 60_000 })).rejects.toThrow(/not connected/i);
  });

  it("pathfind resolves on path_result and rejects on path_error", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();

    const p = client.pathfind("scene-1", [50, 50], [[250, 50]], 0.5);
    const firstIndex = sent.findIndex((s) => JSON.parse(s).type === "pathfind");
    const sentFrame = JSON.parse(sent[firstIndex]);
    expect(sentFrame.type).toBe("pathfind");
    onMessage(
      JSON.stringify({
        type: "path_result",
        request_id: sentFrame.request_id,
        path: [[50, 50], [250, 50]],
        cost: 2,
        arrested: false,
      }),
    );
    await expect(p).resolves.toEqual({ path: [[50, 50], [250, 50]], cost: 2, arrested: false });

    const p2 = client.pathfind("scene-1", [50, 50], [[9999, 9999]], 0.5);
    const sentFrame2 = JSON.parse(sent.find((s, i) => i > firstIndex && JSON.parse(s).type === "pathfind")!);
    onMessage(
      JSON.stringify({
        type: "path_error",
        request_id: sentFrame2.request_id,
        message: "unreachable",
      }),
    );
    await expect(p2).rejects.toThrow("unreachable");
  });

  it("pathfind sends the token id when given, and omits it when absent", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    client.pathfind("scene-1", [0, 0], [[100, 0]], 0.4, "t1");
    const withToken = JSON.parse(sent.find((s) => JSON.parse(s).type === "pathfind")!);
    expect(withToken).toMatchObject({ type: "pathfind", footprint_radius: 0.4, token: "t1" });

    client.pathfind("scene-1", [0, 0], [[100, 0]], 0.4);
    const withoutToken = JSON.parse(sent.filter((s) => JSON.parse(s).type === "pathfind").at(-1)!);
    expect(withoutToken).not.toHaveProperty("token");
  });

  it("move_stream resolves moveRequest AND fires onMoveStream for mover", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();

    const streams: MoveStream[] = [];
    const unsub = client.onMoveStream((s) => streams.push(s));

    const p = client.moveRequest("scene1", "tok1", [[0, 0], [100, 0]]);
    const sentFrame = JSON.parse(sent.find((s) => JSON.parse(s).type === "move_request")!);
    expect(sentFrame.type).toBe("move_request");
    expect(sentFrame.token_id).toBe("tok1");

    onMessage(
      JSON.stringify({
        type: "move_stream",
        request_id: sentFrame.request_id,
        token_id: "tok1",
        mover: "user1",
        scene: "scene1",
        start_server_ms: 1000,
        duration_ms: 500,
        stop: [100, 0],
        samples: [{ t_ms: 0, pos: [0, 0] }, { t_ms: 500, pos: [100, 0] }],
        mover_vision: null,
        cost: 2,
        truncated: true,
      }),
    );

    // Promise resolves with camelCase-mapped MoveStream.
    const result = await p;
    expect(result.tokenId).toBe("tok1");
    // The mapper must copy the flag across; an omitted key here is invisible at parse time.
    expect(result.truncated).toBe(true);
    expect(result.startServerMs).toBe(1000);
    expect(result.durationMs).toBe(500);
    expect(result.stop).toEqual([100, 0]);
    expect(result.samples).toEqual([{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [100, 0] }]);
    expect(result.moverVision).toBeNull();

    // Listener fires for BOTH mover and observers.
    expect(streams).toHaveLength(1);
    expect(streams[0].tokenId).toBe("tok1");
    expect(streams[0].mover).toBe("user1");

    unsub();
  });

  it("move_stream with unknown request_id (observer) fires onMoveStream without rejecting", async () => {
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: () => {}, close: () => {} });
      },
      handlers: noop,
    });
    await client.start();

    const streams: MoveStream[] = [];
    const unsub = client.onMoveStream((s) => streams.push(s));

    // Simulate observer receiving a move_stream with no pending request.
    onMessage(
      JSON.stringify({
        type: "move_stream",
        request_id: "00000000-0000-0000-0000-000000000099",
        token_id: "tok2",
        mover: "user2",
        scene: "scene1",
        start_server_ms: 2000,
        duration_ms: 300,
        stop: [200, 0],
        samples: [{ t_ms: 0, pos: [0, 0] }, { t_ms: 300, pos: [200, 0] }],
        mover_vision: null,
        // A clipped observer's cost is server-nulled (secrecy: mirrors moverVision).
        cost: null,
        // Nulled on the same grounds — the flag would reveal whether anything stopped the
        // token beyond the observer's clipped view.
        truncated: null,
      }),
    );
    await flush();

    // No pending promise was rejected; listener fires with camelCase fields.
    expect(streams).toHaveLength(1);
    expect(streams[0].tokenId).toBe("tok2");
    expect(streams[0].truncated).toBeNull();
    expect(streams[0].mover).toBe("user2");
    expect(streams[0].moverVision).toBeNull();
    expect(streams[0].cost).toBeNull();

    unsub();
  });

  it("moveRequest rejects on move_error", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
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
    await expect(p).rejects.toThrow("move rejected");
  });

  it("moveRequest rejects immediately when there is no live transport", async () => {
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: () => {}, close: () => {} }),
      handlers: noop,
    });
    // Not started → transport is null. A long timeout would otherwise hang.
    await expect(
      client.moveRequest("scene1", "tok1", [[0, 0], [100, 0]]),
    ).rejects.toThrow(/not connected/i);
  });

  it("combat() with selfUserId configured does not resolve on an unrelated event, and resolves on the matching one", async () => {
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      selfUserId: "userA",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: () => {}, close: () => {} });
      },
      handlers: noop,
    });
    await client.start();

    const p = client.combat({ type: "combat_advance", request_id: "r1", combat_id: "c1" });
    let resolved = false;
    void p.then(() => (resolved = true));

    // Unrelated event (different author) must NOT resolve the pending combat entry.
    onMessage(
      JSON.stringify({
        type: "event",
        command: { seq: 1, world_id: "w1", author: "userB", ts: 0, ops: [] },
        intent_id: null,
      }),
    );
    await flush();
    expect(resolved).toBe(false);

    // Matching-author event resolves it.
    onMessage(
      JSON.stringify({
        type: "event",
        command: { seq: 2, world_id: "w1", author: "userA", ts: 0, ops: [] },
        intent_id: null,
      }),
    );
    await expect(p).resolves.toBeUndefined();
  });

  it("combat() without selfUserId configured never resolves from an event frame", async () => {
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: () => {}, close: () => {} });
      },
      handlers: noop,
    });
    await client.start();

    const p = client.combat(
      { type: "combat_advance", request_id: "r1", combat_id: "c1" },
      { timeoutMs: 20 },
    );
    let resolved = false;
    let rejected = false;
    void p.then(
      () => (resolved = true),
      () => (rejected = true),
    );

    onMessage(
      JSON.stringify({
        type: "event",
        command: { seq: 1, world_id: "w1", author: "userA", ts: 0, ops: [] },
        intent_id: null,
      }),
    );
    await flush();
    expect(resolved).toBe(false);
    expect(rejected).toBe(false);

    // Only settles via combat_error or timeout — confirm the timeout path still fires.
    await expect(p).rejects.toThrow(/timeout/i);
  });

  it("sendChatMessage sends a send_message frame with defaulted audience and actor_owner", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    void client.sendChatMessage({ channel: "ic", content: "hello" }).catch(() => {});
    const frame = JSON.parse(sent.find((s) => JSON.parse(s).type === "send_message")!);
    expect(frame).toMatchObject({
      type: "send_message",
      channel: "ic",
      content: "hello",
      actor_owner: null,
      audience: { kind: "public" },
    });
    expect(typeof frame.request_id).toBe("string");
    expect(frame.request_id.length).toBeGreaterThan(0);
  });

  it("sendChatMessage forwards actor_owner and audience when given", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    void client.sendChatMessage({
      channel: "ic",
      content: "hi",
      actorOwner: { kind: "actor", actor_id: "a1" },
      audience: { kind: "whisper", recipients: ["u2"] },
    }).catch(() => {});
    const frame = JSON.parse(sent.find((s) => JSON.parse(s).type === "send_message")!);
    expect(frame).toMatchObject({
      type: "send_message",
      channel: "ic",
      content: "hi",
      actor_owner: { kind: "actor", actor_id: "a1" },
      audience: { kind: "whisper", recipients: ["u2"] },
    });
    expect(typeof frame.request_id).toBe("string");
  });

  it("editChatMessage sends an edit_message frame with a request_id", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    void client.editChatMessage("m1", "edited text").catch(() => {});
    const frame = JSON.parse(sent.find((s) => JSON.parse(s).type === "edit_message")!);
    expect(frame).toMatchObject({ type: "edit_message", message_id: "m1", content: "edited text" });
    expect(typeof frame.request_id).toBe("string");
  });

  it("deleteChatMessage sends a delete_message frame with a request_id", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    void client.deleteChatMessage("m1").catch(() => {});
    const frame = JSON.parse(sent.find((s) => JSON.parse(s).type === "delete_message")!);
    expect(frame).toMatchObject({ type: "delete_message", message_id: "m1" });
    expect(typeof frame.request_id).toBe("string");
  });

  it("recalcRoll sends a recalc_roll frame with a request_id", async () => {
    const sent: string[] = [];
    const client = new WsClient({
      world: "w1",
      connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
      handlers: noop,
    });
    await client.start();

    void client.recalcRoll("msg-1", "roll-1", [{ kind: "remove_dice", ids: [2] }]).catch(() => {});
    const frame = JSON.parse(sent.find((s) => JSON.parse(s).type === "recalc_roll")!);
    expect(frame).toMatchObject({
      type: "recalc_roll",
      message_id: "msg-1",
      roll_id: "roll-1",
      ops: [{ kind: "remove_dice", ids: [2] }],
    });
    expect(typeof frame.request_id).toBe("string");
    expect(frame.request_id.length).toBeGreaterThan(0);
  });

  it("a rejected chat op rejects the correlated promise with the server reason", async () => {
    const sent: string[] = [];
    let onMessage: (d: string) => void = () => {};
    const client = new WsClient({
      world: "w1",
      connect: (h) => {
        onMessage = h.onMessage;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      },
      handlers: noop,
    });
    await client.start();
    const p = client.sendChatMessage({ channel: "ic", content: "hi" });
    const req = JSON.parse(sent.find((s) => JSON.parse(s).type === "send_message")!);
    onMessage(
      JSON.stringify({
        type: "chat_error",
        request_id: req.request_id,
        message: "You are sending messages too quickly. Please wait a moment.",
      }),
    );
    await expect(p).rejects.toThrow(/too quickly/i);
  });

  describe("Hello / seedWatermark", () => {
    it("sends Hello{last_seq: null} as the first frame on a first socket open", async () => {
      const sent: string[] = [];
      const client = new WsClient({
        world: "w1",
        connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
        handlers: noop,
      });
      await client.start();
      await flush();
      expect(sent.length).toBeGreaterThan(0);
      expect(JSON.parse(sent[0])).toEqual({ type: "hello", world: "w1", last_seq: null });
    });

    // The discriminating test: `seedWatermark` pre-advances `nextExpected` BEFORE the first
    // `open()`, exactly as `WorldSession.enter` does with a snapshot's `seq`. A `last_seq`
    // computed from `nextExpected`'s live value would ALWAYS report `Some(...)` at this point
    // (nextExpected is already > 1), never signaling cold start. Only `#hasEverOpened` — an
    // identity fact about the instance's own open history — gets this right.
    it("still sends Hello{last_seq: null} on the first open even after seedWatermark pre-advanced the watermark", async () => {
      const sent: string[] = [];
      const client = new WsClient({
        world: "w1",
        connect: () => Promise.resolve({ send: (d) => sent.push(d), close: () => {} }),
        handlers: noop,
      });
      client.seedWatermark(9); // nextExpected = 10, before any open() at all
      await client.start();
      await flush();
      expect(JSON.parse(sent[0])).toEqual({ type: "hello", world: "w1", last_seq: null });
    });

    it("sends Hello{last_seq: nextExpected - 1} (not null) on a second open of the same instance", async () => {
      const opens: string[][] = [];
      let onClose!: () => void;
      const connect: Connect = (h) => {
        const sent: string[] = [];
        opens.push(sent);
        onClose = h.onClose;
        return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
      };
      const client = new WsClient({
        world: "w1",
        connect,
        handlers: noop,
        sleep: () => Promise.resolve(),
      });
      client.seedWatermark(9); // nextExpected = 10
      await client.start();
      await flush();
      expect(opens.length).toBe(1);
      expect(JSON.parse(opens[0][0])).toEqual({ type: "hello", world: "w1", last_seq: null });

      // Simulate a dropped connection: the transport's own onClose fires, triggering a
      // reconnect (second open) on the SAME WsClient instance.
      onClose();
      await waitFor(() => opens.length === 2);
      expect(JSON.parse(opens[1][0])).toEqual({ type: "hello", world: "w1", last_seq: 9 });
    });

    it("seedWatermark(seq) makes a subsequent Welcome{current_seq: seq} trigger no resync, while current_seq: seq + 3 triggers a small resync_request from seq + 1", async () => {
      const sent: string[] = [];
      let onMessage: (d: string) => void = () => {};
      const client = new WsClient({
        world: "w1",
        connect: (h) => {
          onMessage = h.onMessage;
          return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
        },
        handlers: noop,
      });
      client.seedWatermark(42); // nextExpected = 43
      await client.start();
      await flush();
      onMessage(JSON.stringify({ ...welcomeFrame(), current_seq: 42 }));
      await flush();
      expect(sent.some((s) => (JSON.parse(s) as ClientMsg).type === "resync_request")).toBe(false);

      const opens2: string[] = [];
      let onMessage2: (d: string) => void = () => {};
      const client2 = new WsClient({
        world: "w1",
        connect: (h) => {
          onMessage2 = h.onMessage;
          return Promise.resolve({ send: (d) => opens2.push(d), close: () => {} });
        },
        handlers: noop,
      });
      client2.seedWatermark(42); // nextExpected = 43
      await client2.start();
      await flush();
      onMessage2(JSON.stringify({ ...welcomeFrame(), current_seq: 45 }));
      await flush();
      const resync = opens2.map((s) => JSON.parse(s) as ClientMsg).find((m) => m.type === "resync_request");
      expect(resync).toEqual({ type: "resync_request", from_seq: 43 });
    });

    it("seedWatermark called with a value lower than the current watermark does not move it backward", async () => {
      const sent: string[] = [];
      let onMessage: (d: string) => void = () => {};
      const client = new WsClient({
        world: "w1",
        connect: (h) => {
          onMessage = h.onMessage;
          return Promise.resolve({ send: (d) => sent.push(d), close: () => {} });
        },
        handlers: noop,
      });
      client.seedWatermark(100); // nextExpected = 101
      client.seedWatermark(5); // must NOT move nextExpected backward to 6
      await client.start();
      await flush();
      onMessage(JSON.stringify({ ...welcomeFrame(), current_seq: 100 }));
      await flush();
      expect(sent.some((s) => (JSON.parse(s) as ClientMsg).type === "resync_request")).toBe(false);
    });
  });
});
