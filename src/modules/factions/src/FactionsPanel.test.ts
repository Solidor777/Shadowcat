import { describe, it, expect, vi } from "vitest";
import { tick } from "svelte";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildFactionRegistryDoc, deterministicId, type WireDocument, type WireOperation } from "@shadowcat/core";
import FactionsPanel from "./FactionsPanel.svelte";
import { seedFactionRegistryIfAbsent } from "./seed";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

describe("FactionsPanel field edits", () => {
  it("reads the raw stored value as `old` on a SECOND edit to the same field (OCC regression)", async () => {
    const dispatchIntent = vi.fn();
    const registry = buildFactionRegistryDoc("w1", { f1: { name: "Friendly", color: "#3fb950", stance: "friendly" } }, "reg1");
    const store = gmStoreWith(registry);
    render(FactionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }) });

    const nameInput = screen.getByLabelText("factions.name") as HTMLInputElement;

    // First edit: pristine doc, `old` correctly matches the stored value.
    await fireEvent.change(nameInput, { target: { value: "Allies" } });
    expect(dispatchIntent).toHaveBeenNthCalledWith(1, [
      { op: "update", doc_id: "reg1", changes: [{ path: "/engine/factions/f1/name", old: "Friendly", new: "Allies" }] },
    ]);

    // Apply the first write to the store, as the server would on success, before the second edit.
    store.applyCommand({
      seq: 2, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "update", doc_id: "reg1", changes: [{ path: "/engine/factions/f1/name", old: "Friendly", new: "Allies" }] }],
    });
    await tick();

    // Second edit to the SAME field: `old` must reflect the first write's result, not stay
    // hardcoded at null (a stale `old` gets rejected by the server's field-level OCC check).
    await fireEvent.change(nameInput, { target: { value: "Allies2" } });
    expect(dispatchIntent).toHaveBeenNthCalledWith(2, [
      { op: "update", doc_id: "reg1", changes: [{ path: "/engine/factions/f1/name", old: "Allies", new: "Allies2" }] },
    ]);
  });
});

describe("faction-registry seed", () => {
  it("two GMs entering a brand-new world simultaneously converge on ONE faction-registry, not two", () => {
    const worldId = "world-1";
    const store1 = new DocumentStore(); // GM connection 1
    const store2 = new DocumentStore(); // GM connection 2
    const dispatch1 = vi.fn();
    const dispatch2 = vi.fn();

    seedFactionRegistryIfAbsent(store1, worldId, dispatch1);
    seedFactionRegistryIfAbsent(store2, worldId, dispatch2);

    expect(dispatch1).toHaveBeenCalledTimes(1);
    expect(dispatch2).toHaveBeenCalledTimes(1);
    const doc1 = (dispatch1.mock.calls[0][0] as WireOperation[])[0] as { op: "create"; doc: WireDocument };
    const doc2 = (dispatch2.mock.calls[0][0] as WireOperation[])[0] as { op: "create"; doc: WireDocument };
    expect(doc1.doc.id).toBe(doc2.doc.id); // deterministic id: both racers compute the SAME id

    // The server confirms only the winner's Create (per the singleton create-gate); since both
    // racers used the same id, applying that single confirmed doc converges both stores.
    const cmd = { seq: 1, world_id: worldId, author: "a", ts: 0, ops: [{ op: "create" as const, doc: doc1.doc }] };
    store1.applyCommand(cmd);
    store2.applyCommand(cmd);
    expect(store1.query("faction-registry")).toHaveLength(1);
    expect(store2.query("faction-registry")).toHaveLength(1);
  });

  it("a losing racer gracefully adopts the winning registry instead of erroring", () => {
    const worldId = "world-1";
    const dispatchIntent = vi.fn();
    const store = new DocumentStore();

    // Simulate the server-side create-gate having already rejected this client's own Create,
    // and the winner's registry (same deterministic id) having landed via the event stream.
    const winner = buildFactionRegistryDoc(worldId, { friendly: { name: "Friendly", color: "#3fb950", stance: "friendly" } }, deterministicId(worldId, "faction-registry"));
    store.applyCommand({ seq: 1, world_id: worldId, author: "other", ts: 0, ops: [{ op: "create", doc: winner }] });

    seedFactionRegistryIfAbsent(store, worldId, dispatchIntent);

    expect(dispatchIntent).not.toHaveBeenCalled(); // no error, no duplicate Create attempt
    expect(store.query("faction-registry")).toHaveLength(1); // adopted the existing one
  });
});
