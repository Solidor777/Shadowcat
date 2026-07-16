import { describe, it, expect, vi } from "vitest";
import { tick } from "svelte";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildFactionRegistryDoc, type WireDocument } from "@shadowcat/core";
import FactionsPanel from "./FactionsPanel.svelte";

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
