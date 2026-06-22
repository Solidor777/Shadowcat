import { describe, it, expect } from "vitest";
import { OptimisticClient } from "./optimistic";
import { WsClient } from "./ws-client";
import { MockServer } from "./mock-server";
import type { ClientMsg, WireOperation } from "./wire";

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

async function waitFor(pred: () => boolean, tries = 100): Promise<void> {
  for (let i = 0; i < tries; i++) {
    if (pred()) return;
    await flush();
  }
  throw new Error("waitFor timed out");
}

function createOp(id: string, hp: number): WireOperation {
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
        default: "owner",
        users: {},
        property_overrides: {},
        capabilities: { by_role: {}, by_user: {} },
      },
      embedded: {},
      parent_id: null,
      system: { hp },
      created_at: 0,
      updated_at: 0,
    },
  };
}

function updateOp(id: string, hp: number, prev: number): WireOperation {
  return {
    op: "update",
    doc_id: id,
    changes: [{ path: "/system/hp", old: prev, new: hp }],
  };
}

/** Wire an OptimisticClient to a WsClient against the server, return both plus
 * a helper to apply+send an intent. */
async function connect(server: MockServer, author: string) {
  const oc = new OptimisticClient(author);
  let n = 0;
  const ws = new WsClient({
    connect: server.connector(author),
    handlers: {
      onCommand: (c) => oc.applyCommand(c),
      onReject: (id) => oc.reject(id),
    },
    sleep: () => Promise.resolve(),
  });
  await ws.start();
  await flush();
  const act = (ops: WireOperation[]): string => {
    const id = `${author}-${n++}`;
    oc.applyIntent(id, ops);
    ws.send({ type: "intent", intent_id: id, ops } satisfies ClientMsg);
    return id;
  };
  return { oc, ws, act };
}

describe("OptimisticClient", () => {
  it("predicts then confirms a create", async () => {
    const server = new MockServer();
    const { oc, act } = await connect(server, "u1");

    act([createOp("d1", 10)]);
    // Optimistic: visible immediately, before any server round-trip.
    expect(oc.get("d1")).toBeDefined();
    expect(oc.pendingIntents()).toHaveLength(1);

    await waitFor(() => oc.pendingIntents().length === 0);
    expect(oc.get("d1")).toBeDefined(); // now confirmed into base
    expect(oc.appliedSeq).toBe(1);
  });

  it("rolls back a rejected update", async () => {
    // Reject any update; accept creates.
    const server = new MockServer({
      rejectRule: (ctx) =>
        ctx.ops.some((o) => o.op === "update") ? "forbidden" : null,
    });
    const { oc, act } = await connect(server, "u1");
    act([createOp("d1", 10)]);
    await waitFor(() => oc.pendingIntents().length === 0);

    act([updateOp("d1", 99, 10)]);
    // Optimistically applied.
    expect((oc.get("d1")!.system as { hp: number }).hp).toBe(99);

    await waitFor(() => oc.pendingIntents().length === 0);
    // Rejected → rolled back to the confirmed value.
    expect((oc.get("d1")!.system as { hp: number }).hp).toBe(10);
  });

  it("a peer's event lands in the view while a local intent is pending", async () => {
    const server = new MockServer();
    const a = await connect(server, "ua");
    const b = await connect(server, "ub");

    // A creates a doc; both converge.
    a.act([createOp("shared", 1)]);
    await waitFor(() => b.oc.get("shared") !== undefined);

    // B optimistically updates locally; meanwhile A updates the same doc.
    b.act([updateOp("shared", 50, 1)]);
    a.act([updateOp("shared", 7, 1)]);

    // Both converge to the authoritative last-writer value with no pending left.
    await waitFor(
      () =>
        a.oc.pendingIntents().length === 0 &&
        b.oc.pendingIntents().length === 0,
    );
    const av = (a.oc.get("shared")!.system as { hp: number }).hp;
    const bv = (b.oc.get("shared")!.system as { hp: number }).hp;
    expect(av).toBe(bv);
  });
});
