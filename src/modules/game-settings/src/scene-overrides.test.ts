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

  it("selecting a boolean value on a boolean tri-state select writes a real boolean (not string)", async () => {
    // Round-trip assertion: the fog select dispatches boolean true (not string "true") when
    // the user picks the enabled option on a scene with no prior fog override.
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.fog") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "true" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/fog", old: null, new: true }] },
    ]);
  });

  it("selecting inherit on a boolean override that was null dispatches null (not coercing null to false)", async () => {
    // Exercises the FIX 1 null-as-inherit path: a pre-existing fog=null override must render the
    // inherit option as selected ("" value), and selecting it again must dispatch null (not boolean
    // false, which the old === undefined test would have produced for a null wire value).
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    // Pre-populate scene with fog explicitly set to true; selecting inherit clears it with null.
    const scene = buildSceneDoc("w1", { vision: { fog: true } }, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.fog") as HTMLSelectElement;
    // The control must reflect the current override value ("true"); switching to inherit writes null.
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/fog", old: null, new: null }] },
    ]);
  });

  it("selecting inherit on a previously-set enum override dispatches null to clear it", async () => {
    // Enum (non-boolean) inherit path: selecting the blank option writes null so the
    // nullish-coalesce in resolveSceneSettings falls back to the world default.
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    // Pre-populate the scene with a movementRestriction override already set.
    const scene = buildSceneDoc("w1", { vision: { movementRestriction: "unrestricted" } }, "scene1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws, scene), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.scene.movementRestriction") as HTMLSelectElement;
    // The control reflects the current override ("unrestricted"); selecting "" clears it to null.
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "scene1", changes: [{ path: "/system/vision/movementRestriction", old: null, new: null }] },
    ]);
  });
});
