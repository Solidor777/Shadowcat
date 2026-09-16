import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import {
  DocumentStore,
  buildPlaylistDoc,
  type AudioApi,
  type PlaylistEngine,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import PlaylistSheet from "./PlaylistSheet.svelte";

function engine(tracks: PlaylistEngine["tracks"] = []): PlaylistEngine {
  return { tracks, mode: "sequential", channel: "music", fadeMs: 0 };
}

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

/** Records dispatch calls AND applies updates back onto the store (server-confirm mirror). */
function confirmingDispatch(documents: DocumentStore, calls: unknown[]) {
  let seq = 1;
  return (ops: WireOperation[]) => {
    calls.push(ops);
    const updates = ops.filter((op): op is Extract<WireOperation, { op: "update" }> => op.op === "update");
    if (updates.length === 0) return;
    documents.applyCommand({ seq: ++seq, world_id: "w1", author: "u", ts: 0, ops: updates });
  };
}

const TRACK_A = { asset: "a1", name: null, gain: 1, loop: false };
const TRACK_B = { asset: "b2", name: "Second", gain: 0.5, loop: true };

describe("PlaylistSheet", () => {
  it("edits scalar fields with the real pre-image", async () => {
    const calls: unknown[] = [];
    const doc = buildPlaylistDoc("w1", "Tavern", engine());
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
    });
    const { getByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    await fireEvent.change(getByTestId("playlist-mode"), { target: { value: "shuffle" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: doc.id, changes: [{ path: "/engine/mode", old: "sequential", new: "shuffle" }] }],
    ]);
    await fireEvent.change(getByTestId("playlist-fade"), { target: { value: "1500" } });
    expect(calls[1]).toEqual([
      { op: "update", doc_id: doc.id, changes: [{ path: "/engine/fadeMs", old: 0, new: 1500 }] },
    ]);
  });

  it("a track mutation replaces the WHOLE tracks array with the correct old/new pair", async () => {
    const calls: unknown[] = [];
    const doc = buildPlaylistDoc("w1", "Tavern", engine([TRACK_A, TRACK_B]));
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
    });
    const { getAllByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    await fireEvent.click(getAllByTestId("track-up")[1]);
    expect(calls[0]).toEqual([
      {
        op: "update",
        doc_id: doc.id,
        changes: [{ path: "/engine/tracks", old: [TRACK_A, TRACK_B], new: [TRACK_B, TRACK_A] }],
      },
    ]);
    await fireEvent.click(getAllByTestId("track-remove")[1]);
    expect(calls[1]).toEqual([
      {
        op: "update",
        doc_id: doc.id,
        changes: [{ path: "/engine/tracks", old: [TRACK_B, TRACK_A], new: [TRACK_B] }],
      },
    ]);
  });

  it("the preview button plays the track's asset as a one-shot", async () => {
    const playOneShot = vi.fn();
    const doc = buildPlaylistDoc("w1", "Tavern", engine([TRACK_A]));
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: () => {},
      canEdit: () => true,
      audio: {
        channels: { master: { gain: 1, muted: false } },
        setChannel: () => {},
        unlock: async () => {},
        duck: { addSource: () => ({ set: () => {} }), removeSource: () => {}, gain: 1, depth: 0.7, setDepth: () => {} },
        playOneShot,
        serverNow: () => 0,
        transport: () => {},
      } as unknown as AudioApi,
    });
    const { getByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    await fireEvent.click(getByTestId("track-preview"));
    expect(playOneShot).toHaveBeenCalledWith("a1");
  });

  it("the asset picker is scoped to the audio kind", async () => {
    const calls: unknown[] = [];
    const pickAsset = vi.fn().mockResolvedValue("picked-asset");
    const doc = buildPlaylistDoc("w1", "Tavern", engine([TRACK_A]));
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
      pickAsset: pickAsset as never,
    });
    const { getByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    expect(getByTestId("track-asset").getAttribute("aria-label")).toBe("sheetPlaylist.trackAsset");
    await fireEvent.click(getByTestId("track-asset"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "audio" });
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual([
      {
        op: "update",
        doc_id: doc.id,
        changes: [
          {
            path: "/engine/tracks",
            old: [TRACK_A],
            new: [{ ...TRACK_A, asset: "picked-asset" }],
          },
        ],
      },
    ]);
  });

  it("add track picks an asset first, then appends a row for the pick", async () => {
    const calls: unknown[] = [];
    const pickAsset = vi.fn().mockResolvedValue("picked-asset");
    const doc = buildPlaylistDoc("w1", "Tavern", engine([TRACK_A]));
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
      pickAsset: pickAsset as never,
    });
    const { getByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    await fireEvent.click(getByTestId("playlist-add-track"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "audio" });
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual([
      {
        op: "update",
        doc_id: doc.id,
        changes: [
          {
            path: "/engine/tracks",
            old: [TRACK_A],
            new: [TRACK_A, { asset: "picked-asset", name: null, gain: 1, loop: false }],
          },
        ],
      },
    ]);
  });

  it("the array shrinking below the picked index during the await drops the patch, no throw", async () => {
    const calls: unknown[] = [];
    let resolvePick!: (v: string) => void;
    const pickAsset = vi.fn().mockReturnValue(new Promise<string>((res) => (resolvePick = res)));
    const doc = buildPlaylistDoc("w1", "Tavern", engine([TRACK_A, TRACK_B]));
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
      pickAsset: pickAsset as never,
    });
    const { getAllByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    // Pick the SECOND track (index 1), but don't resolve the pick yet.
    await fireEvent.click(getAllByTestId("track-asset")[1]);
    expect(pickAsset).toHaveBeenCalledWith({ kind: "audio" });

    // The array shrinks to one track (a confirmed remove from elsewhere) while the pick is
    // still in flight — index 1 no longer names a row.
    documents.applyCommand({
      seq: 2,
      world_id: "w1",
      author: "u2",
      ts: 0,
      ops: [{ op: "update", doc_id: doc.id, changes: [{ path: "/engine/tracks", old: [TRACK_A, TRACK_B], new: [TRACK_A] }] }],
    });

    resolvePick("picked-asset");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(calls).toHaveLength(0);
  });

  it("a cancelled add-track pick dispatches nothing (an unassigned row can never be staged)", async () => {
    const calls: unknown[] = [];
    const pickAsset = vi.fn().mockResolvedValue(null);
    const doc = buildPlaylistDoc("w1", "Tavern", engine());
    const documents = storeWith(doc);
    const context = setAppContextForTest({
      documents,
      dispatchIntent: confirmingDispatch(documents, calls),
      canEdit: () => true,
      pickAsset: pickAsset as never,
    });
    const { getByTestId } = render(PlaylistSheet, {
      props: { docId: doc.id, systemPrefix: "/system", close: () => {} },
      context,
    });
    await fireEvent.click(getByTestId("playlist-add-track"));
    expect(pickAsset).toHaveBeenCalledWith({ kind: "audio" });
    // Let any (unexpected) resolve-then-dispatch microtask chain settle.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(calls).toHaveLength(0);
  });
});
