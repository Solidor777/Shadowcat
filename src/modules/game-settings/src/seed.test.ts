import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

describe("game-settings seed", () => {
  it("GM seeds world-settings, light-gradation, vision-modes once", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    const created = dispatchIntent.mock.calls.flatMap((c) => c[0]).map((op: { doc: { doc_type: string } }) => op.doc.doc_type);
    expect(created).toContain("world-settings");
    expect(created).toContain("light-gradation");
    expect(created).toContain("vision-modes");
  });

  it("non-GM seeds nothing", () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), dispatchIntent }),
    });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});
