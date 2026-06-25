import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("world defaults editor", () => {
  it("changing movement restriction dispatches a JSON-pointer update", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.movementRestriction") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "revealed" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/system/scene/movementRestriction", old: null, new: "revealed" }] },
    ]);
  });

  it("changing diagonal rule dispatches the pathfinding pointer", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.diagonalRule") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "alternating" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/system/pathfinding/diagonalRule", old: null, new: "alternating" }] },
    ]);
  });
});
