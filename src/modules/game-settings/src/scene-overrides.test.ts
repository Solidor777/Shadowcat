import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, buildSceneDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("per-scene overrides", () => {
  it("setting movement restriction override writes to the selected scene doc", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.movementRestriction") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "unrestricted" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/movementRestriction", old: null, new: "unrestricted" }] },
    ]);
  });

  it("setting grid distance per-cell writes to the scene grid", async () => {
    const dispatchIntent = vi.fn();
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(buildWorldSettingsDoc("w1", undefined, "ws1"), scene), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.scene.distancePerCell") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1.5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/grid/distance", old: null, new: { perCell: 1.5, unit: "ft" } }] },
    ]);
  });
});
