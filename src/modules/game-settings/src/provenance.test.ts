import { render, screen, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi } from "vitest";
import { DocumentStore, buildWorldSettingsDoc, buildSystemDefaultsDoc, deterministicId, SYSTEM_DEFAULTS_DOC_TYPE, type WireDocument } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("settings provenance", () => {
  it("shows which layer supplies each world default", () => {
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: false } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(sd, ws), dispatchIntent: vi.fn() }) });
    expect(screen.getByTestId("provenance:scene.fog").textContent).toContain("gameSettings.source.world");
  });

  it("reset to system default writes the system-resolved value into the required world leaf", async () => {
    const dispatchIntent = vi.fn();
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: false } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(sd, ws), dispatchIntent }) });
    await fireEvent.click(screen.getByLabelText("gameSettings.resetToSystem:scene.fog"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/scene/fog", old: true, new: false }] },
    ]);
  });
});
