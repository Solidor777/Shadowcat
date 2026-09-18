import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildWorldSettingsDoc, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

// Suppress listAssets fetch: GameSettingsPanel's dice-sound-picker $effect calls listAssets
// unconditionally on mount, which hits /api/... in jsdom (EmissionEditor.test.ts precedent).
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("world audio overlay editor", () => {
  it("spatial/occlusion/throughWallGain each dispatch a whole /engine/audio object with the real pre-image", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", { audio: { spatial: true, occlusion: null, throughWallGain: null } }, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    await fireEvent.change(screen.getByLabelText("gameSettings.audio.spatial"), { target: { value: "false" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "ws1",
        changes: [
          {
            path: "/engine/audio",
            old: { spatial: true, occlusion: null, throughWallGain: null },
            new: { spatial: false, occlusion: null, throughWallGain: null },
          },
        ],
      },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.audio.occlusion"), { target: { value: "none" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "ws1",
        changes: [
          {
            path: "/engine/audio",
            old: { spatial: true, occlusion: null, throughWallGain: null },
            new: { spatial: true, occlusion: "none", throughWallGain: null },
          },
        ],
      },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.audio.throughWallGain"), { target: { value: "0.5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "ws1",
        changes: [
          {
            path: "/engine/audio",
            old: { spatial: true, occlusion: null, throughWallGain: null },
            new: { spatial: true, occlusion: null, throughWallGain: 0.5 },
          },
        ],
      },
    ]);
  });

  it("the default option round-trips to null, clearing the leaf", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", { audio: { spatial: false, occlusion: null, throughWallGain: 0.5 } }, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    await fireEvent.change(screen.getByLabelText("gameSettings.audio.spatial"), { target: { value: "" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "ws1",
        changes: [
          {
            path: "/engine/audio",
            old: { spatial: false, occlusion: null, throughWallGain: 0.5 },
            new: { spatial: null, occlusion: null, throughWallGain: 0.5 },
          },
        ],
      },
    ]);
  });

  it("an absent overlay dispatches old: null with a partial value (omitted leaves fall back)", async () => {
    const dispatchIntent = vi.fn();
    const ws = buildWorldSettingsDoc("w1", {}, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(ws), dispatchIntent }) });

    await fireEvent.change(screen.getByLabelText("gameSettings.audio.spatial"), { target: { value: "false" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "ws1",
        changes: [{ path: "/engine/audio", old: null, new: { spatial: false } }],
      },
    ]);
  });

  it("the audio fieldset is hidden for a non-GM", () => {
    const ws = buildWorldSettingsDoc("w1", {}, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(ws), dispatchIntent: vi.fn() }) });
    expect(screen.queryByLabelText("gameSettings.audio.spatial")).toBeNull();
    expect(screen.queryByLabelText("gameSettings.audio.occlusion")).toBeNull();
    expect(screen.queryByLabelText("gameSettings.audio.throughWallGain")).toBeNull();
  });
});
