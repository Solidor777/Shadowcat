import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, buildSceneDoc, buildResourceRegistryDoc, type CombatDefaults, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

function ws(combat: CombatDefaults | null = null): WireDocument {
  return buildWorldSettingsDoc("w1", combat ? { combat } : {}, "ws1");
}

function scene(combat: CombatDefaults | null = null): WireDocument {
  return buildSceneDoc("w1", combat ? { combat } : {}, "s1");
}

describe("CombatSceneOverrides (per-scene chain editor)", () => {
  it("selecting an interpretation override dispatches a whole-object write with the raw pre-image", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene({ enforcement: "warn" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.scene.interpretation") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "spaces" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "s1", changes: [{ path: "/engine/combat", old: { enforcement: "warn" }, new: { enforcement: "warn", interpretation: "spaces" } }] },
    ]);
  });

  it("selecting Inherit removes the key, collapsing to null when it was the last authored leaf", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene({ interpretation: "spaces" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.scene.interpretation") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "__inherit" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "s1", changes: [{ path: "/engine/combat", old: { interpretation: "spaces" }, new: null }] },
    ]);
  });

  it("movementResource: selecting None writes null; selecting Inherit removes the key", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene({ movementResource: "gold" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.scene.movementResource") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "__none" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "s1", changes: [{ path: "/engine/combat", old: { movementResource: "gold" }, new: { movementResource: null } }] },
    ]);
    await fireEvent.change(sel, { target: { value: "__inherit" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "s1", changes: [{ path: "/engine/combat", old: { movementResource: "gold" }, new: null }] },
    ]);
  });

  it("the movement-resource select lists the resource registry's keys", async () => {
    const store = storeWith(ws(), scene(), buildResourceRegistryDoc("w1", { gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } }));
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn() }) });
    const sel = screen.getByLabelText("gameSettings.combat.scene.movementResource") as HTMLSelectElement;
    expect([...sel.options].map((o) => o.value)).toContain("gold");
  });

  it("a lifecycle field of \"1\" writes the number 1", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene()), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.combat.scene.onCombatEnd") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "s1", changes: [{ path: "/engine/combat", old: null, new: { effectLifecycle: { onCombatEnd: 1 } } }] },
    ]);
  });

  it("an invalid lifecycle formula shows the inline error and writes nothing", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene()), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.combat.scene.onCombatEnd") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1 +" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("gameSettings.resources.invalid")).toBeTruthy();
  });

  it("the provenance hint reflects a scene override, and the effective-rules summary in CombatSettings updates through the shared store", async () => {
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene({ enforcement: "hard" })), dispatchIntent: vi.fn() }) });
    expect(screen.getByTestId("provenance:combat.scene.enforcement").textContent).toBe("gameSettings.source.scene");
    const cell = screen.getByTestId("gameSettings:combat-effective-combat.enforcement");
    expect(cell.textContent).toBe('"hard"');
  });
});
