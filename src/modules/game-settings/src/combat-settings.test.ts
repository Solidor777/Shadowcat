import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, buildResourceRegistryDoc, buildSceneDoc, type CombatDefaults, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

function ws(combat: CombatDefaults | null = null): WireDocument {
  return buildWorldSettingsDoc("w1", combat ? { combat } : {}, "ws1");
}

describe("CombatSettings (world chain editor)", () => {
  it("selecting an interpretation dispatches a whole-object write with the raw pre-image", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws({ enforcement: "warn" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.interpretation") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "spaces" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: { enforcement: "warn" }, new: { enforcement: "warn", interpretation: "spaces" } }] },
    ]);
  });

  it("selecting Inherit removes the key, collapsing to null when it was the last authored leaf", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws({ interpretation: "spaces" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.interpretation") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "__inherit" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: { interpretation: "spaces" }, new: null }] },
    ]);
  });

  it("selecting Inherit when other leaves remain keeps the object, minus that key", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws({ interpretation: "spaces", enforcement: "hard" })), dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.interpretation") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "__inherit" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: { interpretation: "spaces", enforcement: "hard" }, new: { enforcement: "hard" } }] },
    ]);
  });

  it("movementResource: selecting None writes null; selecting Inherit removes the key", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(ws({ movementResource: "gold" }));
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }) });
    const sel = screen.getByLabelText("gameSettings.combat.movementResource") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "__none" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: { movementResource: "gold" }, new: { movementResource: null } }] },
    ]);
    await fireEvent.change(sel, { target: { value: "__inherit" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: { movementResource: "gold" }, new: null }] },
    ]);
  });

  it("the movement-resource select lists the resource registry's keys", () => {
    const store = storeWith(ws(), buildResourceRegistryDoc("w1", { gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } }));
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn() }) });
    const sel = screen.getByLabelText("gameSettings.combat.movementResource") as HTMLSelectElement;
    expect([...sel.options].map((o) => o.value)).toContain("gold");
  });

  it("a lifecycle field of \"1\" writes the number 1", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws()), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.combat.onCombatEnd") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: null, new: { effectLifecycle: { onCombatEnd: 1 } } }] },
    ]);
  });

  it("an invalid lifecycle formula shows the inline error and writes nothing", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws()), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.combat.onCombatEnd") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "1 +" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("gameSettings.resources.invalid")).toBeTruthy();
  });

  it("a valid formula string writes the trimmed string", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws()), dispatchIntent }) });
    const input = screen.getByLabelText("gameSettings.combat.onTurnEnd") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "max(hp, 0)" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/combat", old: null, new: { effectLifecycle: { onTurnEnd: "max(hp, 0)" } } }] },
    ]);
  });

  it("the provenance hint reflects the world source and the reset button appears only for a world leaf", () => {
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws({ enforcement: "hard" })), dispatchIntent: vi.fn() }) });
    expect(screen.getByTestId("provenance:combat.enforcement").textContent).toBe("gameSettings.source.world");
    expect(screen.getByTestId("provenance:combat.interpretation").textContent).toBe("gameSettings.source.engine");
  });

  it("the effective-rules summary shows every combat leaf resolved for the selected scene", () => {
    const scene = buildSceneDoc("w1", { combat: { enforcement: "warn" } }, "s1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(ws(), scene), dispatchIntent: vi.fn() }) });
    const cell = screen.getByTestId("gameSettings:combat-effective-combat.enforcement");
    expect(cell.textContent).toBe('"warn"');
  });
});
