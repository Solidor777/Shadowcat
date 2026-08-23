import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc, buildDiceSettingsDoc, buildChatSettingsDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

describe("game-settings seed", () => {
  it("GM seeds world-settings, light-gradation, vision-modes, dice-settings, chat-settings once", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops).toHaveLength(5);
    const created = ops.map((op) => (op as { op: "create"; doc: { doc_type: string } }).doc.doc_type);
    expect(created).toContain("world-settings");
    expect(created).toContain("light-gradation");
    expect(created).toContain("vision-modes");
    expect(created).toContain("dice-settings");
    expect(created).toContain("chat-settings");
    const diceOp = ops.find((op) => (op as { op: "create"; doc: { doc_type: string } }).doc.doc_type === "dice-settings") as
      { op: "create"; doc: { engine: unknown } };
    expect(diceOp.doc.engine).toEqual({ mode: "total", direction: "high_wins", channel_overrides: {} });
    const chatOp = ops.find((op) => (op as { op: "create"; doc: { doc_type: string } }).doc.doc_type === "chat-settings") as
      { op: "create"; doc: { engine: unknown } };
    expect(chatOp.doc.engine).toEqual({ markdown: null, html: null, images: null, hyperlinks: null, emails: null, link_previews: null });
  });

  it("non-GM seeds nothing", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("does not seed when all five config docs already exist", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(
      buildWorldSettingsDoc("w1"),
      buildLightGradationDoc("w1"),
      buildVisionModesDoc("w1"),
      buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }),
      buildChatSettingsDoc("w1", { markdown: null, html: null, images: null, hyperlinks: null, emails: null, link_previews: null }),
    );
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }),
    });
    await Promise.resolve();
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});
