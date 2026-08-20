import { describe, it, expect, vi } from "vitest";
import { OptimisticClient } from "./optimistic";
import { WsClient } from "./ws-client";
import { MockServer } from "./mock-server";
import { buildFactionRegistryDoc, deterministicId } from "./scene-docs";
import type { ClientMsg, WireDocument, WireOperation } from "./wire";

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
      name: null,
      source: null,
      owner: null,
      permissions: {
        default: "owner",
        users: {},
        property_overrides: {},
        capabilities: { by_role: {}, by_user: {} },
        gm_role: null,
      },
      embedded: {},
      parent_id: null,
      system: { hp },
      created_at: 0,
      updated_at: 0,
    },
  };
}

function doc(id: string, hp: number): WireDocument {
  const op = createOp(id, hp);
  if (op.op !== "create") throw new Error("unreachable: createOp always returns a create op");
  return op.doc;
}

function updateOp(id: string, hp: number, prev: number): WireOperation {
  return {
    op: "update",
    doc_id: id,
    changes: [{ path: "/system/hp", old: prev, new: hp }],
  };
}

function removeOp(id: string, prevHp: number): WireOperation {
  return {
    op: "update",
    doc_id: id,
    changes: [{ path: "/system/hp", old: prevHp, new: null, remove: true }],
  };
}

/** Wire an OptimisticClient to a WsClient against the server, return both plus
 * a helper to apply+send an intent. */
async function connect(server: MockServer, author: string) {
  const oc = new OptimisticClient(author);
  let n = 0;
  const ws = new WsClient({
    world: "w1",
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

  it("optimistically removes a key as GENUINE absence (view + confirmed base)", async () => {
    const server = new MockServer();
    const { oc, act } = await connect(server, "u1");
    act([createOp("d1", 10)]);
    await waitFor(() => oc.pendingIntents().length === 0);

    act([removeOp("d1", 10)]);
    // Optimistic view: hp is genuinely absent immediately, not present-as-null.
    expect("hp" in (oc.get("d1")!.system as Record<string, unknown>)).toBe(false);

    await waitFor(() => oc.pendingIntents().length === 0);
    // Confirmed into base: still absent (the confirmed command applies the same removal).
    expect("hp" in (oc.get("d1")!.system as Record<string, unknown>)).toBe(false);
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

  it("a deterministic-id singleton Create race rolls the loser back and converges both clients on the winner's confirmed doc", async () => {
    const worldId = "world-1";
    const id = deterministicId(worldId, "faction-registry");
    // Simulates the server's doc_type-scoped singleton create-gate: since both racers compute
    // the same deterministic id, the gate degenerates to a plain id-conflict check here.
    const created = new Set<string>();
    const server = new MockServer({
      rejectRule: (ctx) => {
        for (const op of ctx.ops) {
          if (op.op === "create") {
            if (created.has(op.doc.id)) return "conflict";
            created.add(op.doc.id);
          }
        }
        return null;
      },
    });
    const a = await connect(server, "gmA");
    const b = await connect(server, "gmB");

    // Same id, but envelope() stamps created_at/updated_at via Date.now() per call, so the two
    // racers' payloads genuinely differ in content — only the id is guaranteed identical.
    const seed = { friendly: { name: "Friendly", color: "#3fb950", stance: "friendly" as const } };
    a.act([{ op: "create", doc: buildFactionRegistryDoc(worldId, seed, id) }]);
    b.act([{ op: "create", doc: buildFactionRegistryDoc(worldId, seed, id) }]);

    await waitFor(
      () => a.oc.pendingIntents().length === 0 && b.oc.pendingIntents().length === 0,
    );

    // Exactly one doc for that id on each side, no phantom/duplicate/stale entry, and both
    // converge on the SAME winner (the loser's rolled-back prediction leaves no residue).
    expect(a.oc.query("faction-registry")).toHaveLength(1);
    expect(b.oc.query("faction-registry")).toHaveLength(1);
    expect(a.oc.get(id)).toBeDefined();
    expect(b.oc.get(id)).toEqual(a.oc.get(id));
  });

  // Bug: a throwing optimistic op wedged the write queue. setPointer throws when a change's
  // path descends into a scalar leaf (here: "/system/hp/value" where hp is a number, not a
  // container) — rebuildView must isolate that intent's failure rather than letting it abort
  // the whole rebuild and re-throw on every subsequent applyIntent/applyCommand/reject.
  it("isolates a throwing optimistic intent so a later good intent's effect still lands (queue not wedged)", () => {
    const warn = vi.fn();
    const oc = new OptimisticClient("u1", { debug() {}, warn, error() {} });
    oc.applyIntent("i0", [createOp("d1", 10)]);
    expect(oc.pendingIntents()).toEqual(["i0"]);

    const badOp: WireOperation = {
      op: "update",
      doc_id: "d1",
      changes: [{ path: "/system/hp/value", old: null, new: 1 }],
    };
    expect(() => oc.applyIntent("bad", [badOp])).not.toThrow();
    // The failed intent stays pending — only a server confirm/reject removes it.
    expect(oc.pendingIntents()).toEqual(["i0", "bad"]);
    expect(warn).toHaveBeenCalledWith(
      expect.stringContaining("dropping optimistic intent"),
      expect.objectContaining({ intentId: "bad" }),
    );

    const goodOp: WireOperation = {
      op: "update",
      doc_id: "d1",
      changes: [{ path: "/system/mp", old: null, new: 5 }],
    };
    expect(() => oc.applyIntent("good", [goodOp])).not.toThrow();
    // Good intent's effect IS present in the view, despite the bad intent queued before it.
    expect((oc.get("d1")!.system as { mp: number }).mp).toBe(5);
    expect(oc.pendingIntents()).toEqual(["i0", "bad", "good"]);
  });

  it("a later applyCommand/reject still works after a bad intent is queued", () => {
    const oc = new OptimisticClient("u1");
    oc.applyIntent("i0", [createOp("d1", 10)]);
    oc.applyIntent("bad", [
      {
        op: "update",
        doc_id: "d1",
        changes: [{ path: "/system/hp/value", old: null, new: 1 }],
      },
    ]);

    // A confirming command for i0 (FIFO) must not throw despite "bad" still pending.
    expect(() =>
      oc.applyCommand({
        seq: 1,
        world_id: "test-world",
        author: "u1",
        ts: 0,
        ops: [createOp("d1", 10)],
      }),
    ).not.toThrow();
    expect(oc.pendingIntents()).toEqual(["bad"]);

    // Rejecting the bad intent must not throw either, and clears it.
    expect(() => oc.reject("bad")).not.toThrow();
    expect(oc.pendingIntents()).toEqual([]);
  });

  describe("seedDocuments", () => {
    it("populates get(id) for every seeded document", () => {
      const oc = new OptimisticClient("u1");
      oc.seedDocuments([doc("d1", 10), doc("d2", 5)]);
      expect(oc.get("d1")).toBeDefined();
      expect(oc.get("d2")).toBeDefined();
      expect((oc.get("d1")!.system as { hp: number }).hp).toBe(10);
    });

    it("fires the subscribe listener exactly once for the whole batch", () => {
      const oc = new OptimisticClient("u1");
      let calls = 0;
      oc.subscribe(() => calls++);
      oc.seedDocuments([doc("d1", 1), doc("d2", 2), doc("d3", 3)]);
      expect(calls).toBe(1);
    });

    it("replaces prior state wholesale rather than merging", () => {
      const oc = new OptimisticClient("u1");
      oc.seedDocuments([doc("d1", 1), doc("d2", 2)]);
      expect(oc.get("d1")).toBeDefined();
      // A naive "merge into existing base" implementation would still have d1
      // defined here; a wholesale replace does not.
      oc.seedDocuments([doc("d2", 2), doc("d3", 3)]);
      expect(oc.get("d1")).toBeUndefined();
      expect(oc.get("d2")).toBeDefined();
      expect(oc.get("d3")).toBeDefined();
    });
  });
});
