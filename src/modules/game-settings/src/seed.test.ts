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

describe("server-seeded config singletons", () => {
  it("a GM mount on an empty store dispatches no config-singleton creates (the server seeds them at world creation/join)", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    expect(dispatchIntent).not.toHaveBeenCalled();
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
