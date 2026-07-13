import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildDiceSettingsDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("dice settings editor", () => {
  it("changing mode dispatches a JSON-pointer update", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins" }, "dice1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.mode") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "success_count" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "dice1", changes: [{ path: "/system/mode", old: "total", new: "success_count" }] },
    ]);
  });

  it("changing direction dispatches a JSON-pointer update", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins" }, "dice1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.direction") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "low_wins" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "dice1", changes: [{ path: "/system/direction", old: "high_wins", new: "low_wins" }] },
    ]);
  });

  it("selects reflect the stored doc values", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "success_count", direction: "low_wins" }, "dice1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice), dispatchIntent }) });

    const modeSel = screen.getByLabelText("gameSettings.dice.mode") as HTMLSelectElement;
    const dirSel = screen.getByLabelText("gameSettings.dice.direction") as HTMLSelectElement;
    expect(modeSel.value).toBe("success_count");
    expect(dirSel.value).toBe("low_wins");
  });

  it("is not rendered for a non-GM", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins" }, "dice1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(dice), dispatchIntent }) });

    expect(screen.queryByLabelText("gameSettings.dice.mode")).toBeNull();
  });

  // Insurance: pins the panel's rendered option VALUES to the exact DiceSettingsSystem literal
  // unions (chat-docs.ts DiceSettingsSystem.mode / .direction). A drift here (e.g. a new mode
  // added to the server body but not the panel, or vice versa) must fail loudly rather than
  // silently omit an option.
  it("mode/direction selects expose exactly the DiceSettingsSystem literal option sets", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins" }, "dice1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice), dispatchIntent }) });

    const modeSel = screen.getByLabelText("gameSettings.dice.mode") as HTMLSelectElement;
    const dirSel = screen.getByLabelText("gameSettings.dice.direction") as HTMLSelectElement;
    const modeValues = Array.from(modeSel.options).map((o) => o.value);
    const dirValues = Array.from(dirSel.options).map((o) => o.value);

    expect(modeValues).toEqual(["total", "success_count"]);
    expect(dirValues).toEqual(["high_wins", "low_wins"]);
  });
});
