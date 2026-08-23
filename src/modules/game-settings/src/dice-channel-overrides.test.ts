import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildDiceSettingsDoc, buildChannelRegistryDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("per-channel dice-settings editor", () => {
  it("renders nothing when the channel registry has no channels", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", {}, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    // setAppContextForTest's default `t` returns the literal key (not
    // translated copy), so the section heading renders as this exact string.
    expect(screen.queryByText("gameSettings.dice.channelOverrides")).toBeNull();
  });

  it("renders one row per registered channel, defaulting to inherit", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" }, ic: { name: "In Character" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const generalSel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    const icSel = screen.getByLabelText("gameSettings.dice.channelOverride.ic") as HTMLSelectElement;
    expect(generalSel.value).toBe("");
    expect(icSel.value).toBe("");
    expect(screen.queryByLabelText("gameSettings.dice.channelOverride.general.mode")).toBeNull();
  });

  it("selecting Custom seeds mode/direction from the world default and dispatches a create", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "override" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "dice1", changes: [{ path: "/engine/channel_overrides/general", old: null, new: { mode: "total", direction: "high_wins" } }] },
    ]);
  });

  it("editing mode on an existing override writes the FULL override object (full replacement)", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const modeSel = screen.getByLabelText("gameSettings.dice.channelOverride.general.mode") as HTMLSelectElement;
    await fireEvent.change(modeSel, { target: { value: "success_count" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "success_count", direction: "high_wins" },
        }],
      },
    ]);
  });

  it("editing direction on an existing override writes the FULL override object", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const dirSel = screen.getByLabelText("gameSettings.dice.channelOverride.general.direction") as HTMLSelectElement;
    await fireEvent.change(dirSel, { target: { value: "low_wins" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "total", direction: "low_wins" },
        }],
      },
    ]);
  });

  it("switching back to Inherit removes the key via a whole-map replace", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "success_count", direction: "low_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.dice.channelOverride.general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides",
          old: { general: { mode: "success_count", direction: "low_wins" } },
          new: {},
        }],
      },
    ]);
  });

  it("is not rendered for a non-GM", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent }) });

    expect(screen.queryByLabelText("gameSettings.dice.channelOverride.general")).toBeNull();
  });
});
