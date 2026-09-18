import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildSceneDoc,
  buildWorldSettingsDoc,
  type WireDocument,
  type WireOperation,
  type WireSearchHit,
} from "@shadowcat/core";
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

function playlistHit(id: string, name: string): WireSearchHit {
  return {
    document: {
      id,
      scope: { kind: "world", world_id: "w1" },
      doc_type: "playlist",
      schema_version: 1,
      name,
      source: null,
      owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {},
      parent_id: null,
      engine: { tracks: [], mode: "sequential", channel: "music", fadeMs: 0 },
      system: {},
      created_at: 0,
      updated_at: 0,
    } as WireDocument,
    snippet: "",
    score: 1,
  } as WireSearchHit;
}

describe("scene ambience picker", () => {
  it("typing a query searches playlists; clicking a hit dispatches /engine/ambience with the real pre-image and gain 1", async () => {
    const dispatchIntent = vi.fn();
    const searchDocuments = vi.fn().mockImplementation((_q, _o, onUpdate) => {
      onUpdate([playlistHit("pl1", "Tavern Loop")]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const scene = buildSceneDoc("w1", {}, "scene1");
    const ws = buildWorldSettingsDoc("w1", { activeScene: "scene1" }, "ws1");
    render(GameSettingsPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: gmStoreWith(scene, ws),
        dispatchIntent,
        searchDocuments: searchDocuments as never,
      }),
    });

    await fireEvent.input(screen.getByLabelText("gameSettings.scene.ambienceSearch"), { target: { value: "tav" } });
    await waitFor(() =>
      expect(searchDocuments).toHaveBeenCalledWith(
        "tav",
        expect.objectContaining({ docTypes: ["playlist"] }),
        expect.anything(),
      ),
    );
    await fireEvent.click(await screen.findByRole("button", { name: "Tavern Loop" }));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "scene1",
        changes: [{ path: "/engine/ambience", old: null, new: { playlist: "pl1", gain: 1 } }],
      },
    ]);
  });

  it("the gain slider dispatches the existing playlist unchanged; clear dispatches null with the real pre-image", async () => {
    const scene = buildSceneDoc("w1", { ambience: { playlist: "pl1", gain: 1 } }, "scene1");
    const ws = buildWorldSettingsDoc("w1", { activeScene: "scene1" }, "ws1");
    const documents = gmStoreWith(scene, ws);
    let seq = 1;
    const dispatchIntent = vi.fn().mockImplementation((ops: WireOperation[]) => {
      const updates = ops.filter((op): op is Extract<WireOperation, { op: "update" }> => op.op === "update");
      if (updates.length > 0) {
        documents.applyCommand({ seq: ++seq, world_id: "w1", author: "a", ts: 0, ops: updates });
      }
    });
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents, dispatchIntent }),
    });

    // The search UI is replaced by the playlist+gain+clear UI once ambience is set.
    expect(screen.queryByLabelText("gameSettings.scene.ambienceSearch")).toBeNull();
    await fireEvent.change(screen.getByLabelText("gameSettings.scene.ambienceGain"), { target: { value: "0.4" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "scene1",
        changes: [
          { path: "/engine/ambience", old: { playlist: "pl1", gain: 1 }, new: { playlist: "pl1", gain: 0.4 } },
        ],
      },
    ]);

    await fireEvent.click(screen.getByRole("button", { name: "gameSettings.scene.ambienceClear" }));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "scene1",
        changes: [{ path: "/engine/ambience", old: { playlist: "pl1", gain: 0.4 }, new: null }],
      },
    ]);
  });
});
