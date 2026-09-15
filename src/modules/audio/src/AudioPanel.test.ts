import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import {
  DocumentStore,
  type AudioApi,
  type AudioStateEngine,
  type PlayingTrack,
  type WireDocument,
} from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import AudioPanel from "./AudioPanel.svelte";

function track(id: string, over: Partial<PlayingTrack> = {}): PlayingTrack {
  return {
    id,
    playlist: "pl1",
    trackIndex: 0,
    asset: "asset-1",
    channel: "music",
    gain: 1,
    loop: false,
    startedAt: 0,
    pausedAt: null,
    ...over,
  };
}

function audioStateDoc(playing: PlayingTrack[]): WireDocument {
  return {
    id: "as1",
    scope: { kind: "world", world_id: "w1" },
    doc_type: "audio-state",
    schema_version: 1,
    name: null,
    source: null,
    owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {},
    parent_id: null,
    engine: { playing, shuffleSeed: 0 } satisfies AudioStateEngine as unknown,
    system: {},
    created_at: 0,
    updated_at: 0,
  } as WireDocument;
}

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  if (docs.length > 0) {
    s.applyCommand({
      seq: 1,
      world_id: "w1",
      author: "u",
      ts: 0,
      ops: docs.map((doc) => ({ op: "create" as const, doc })),
    });
  }
  return s;
}

function audioFixture(over: Partial<AudioApi> = {}): AudioApi {
  return {
    channels: {
      master: { gain: 1, muted: false },
      music: { gain: 1, muted: false },
      ambience: { gain: 1, muted: false },
      sfx: { gain: 1, muted: false },
      ui: { gain: 1, muted: false },
    },
    setChannel: vi.fn(),
    unlock: vi.fn().mockResolvedValue(undefined),
    duck: { addSource: () => ({ set: () => {} }), removeSource: () => {}, gain: 1, depth: 0.7, setDepth: () => {} },
    playOneShot: vi.fn(),
    serverNow: () => 10_000,
    transport: vi.fn(),
    ...over,
  } as unknown as AudioApi;
}

describe("AudioPanel", () => {
  it("renders transport buttons for a GM and hides them for a player", () => {
    const documents = storeWith(audioStateDoc([track("e1")]));
    const { unmount } = render(AudioPanel, {
      context: setAppContextForTest({ role: "gm", documents, audio: audioFixture() }),
    });
    expect(screen.getByTestId("playing-pause")).toBeTruthy();
    expect(screen.getByTestId("playing-stop")).toBeTruthy();
    expect(screen.getByTestId("stop-all")).toBeTruthy();
    unmount();
    render(AudioPanel, {
      context: setAppContextForTest({ role: "player", documents, audio: audioFixture() }),
    });
    expect(screen.getByTestId("playing-row")).toBeTruthy();
    expect(screen.queryByTestId("playing-pause")).toBeNull();
    expect(screen.queryByTestId("playing-stop")).toBeNull();
    expect(screen.queryByTestId("stop-all")).toBeNull();
  });

  it("data-audio-playing counts only unpaused entries", () => {
    const { unmount } = render(AudioPanel, {
      context: setAppContextForTest({ role: "gm", documents: storeWith(), audio: audioFixture() }),
    });
    expect(screen.getByTestId("audio-panel").getAttribute("data-audio-playing")).toBe("0");
    unmount();

    render(AudioPanel, {
      context: setAppContextForTest({
        role: "gm",
        documents: storeWith(audioStateDoc([track("e1"), track("e2", { pausedAt: 5_000 })])),
        audio: audioFixture(),
      }),
    });
    // One live (unpaused) of two — a paused entry never raises the count.
    expect(screen.getByTestId("audio-panel").getAttribute("data-audio-playing")).toBe("1");
  });

  it("channel slider and mute call setChannel with the expected patches", async () => {
    const audio = audioFixture();
    render(AudioPanel, {
      context: setAppContextForTest({ role: "gm", documents: storeWith(), audio }),
    });
    await fireEvent.change(screen.getByTestId("channel-gain-music"), { target: { value: "0.4" } });
    expect(audio.setChannel).toHaveBeenCalledWith("music", { gain: 0.4 });
    await fireEvent.click(screen.getByTestId("channel-mute-sfx"));
    expect(audio.setChannel).toHaveBeenCalledWith("sfx", { muted: true });
  });

  it("transport buttons send the expected ops; seek reports position_ms", async () => {
    const audio = audioFixture();
    render(AudioPanel, {
      context: setAppContextForTest({
        role: "gm",
        documents: storeWith(audioStateDoc([track("e1")])),
        audio,
      }),
    });
    await fireEvent.click(screen.getByTestId("playing-pause"));
    expect(audio.transport).toHaveBeenCalledWith({ type: "pause", id: "e1" });
    await fireEvent.click(screen.getByTestId("playing-next"));
    expect(audio.transport).toHaveBeenCalledWith({ type: "next", id: "e1" });
    await fireEvent.change(screen.getByTestId("playing-seek"), { target: { value: "30" } });
    expect(audio.transport).toHaveBeenCalledWith({ type: "seek", id: "e1", position_ms: 30_000 });
    await fireEvent.click(screen.getByTestId("stop-all"));
    expect(audio.transport).toHaveBeenCalledWith({ type: "stop_all" });
  });

  it("live search sends docTypes: [playlist] and create dispatches a playlist document then opens its sheet", async () => {
    const searchDocuments = vi.fn().mockResolvedValue({ unsubscribe: () => {} });
    const calls: unknown[] = [];
    const openDocument = vi.fn();
    render(AudioPanel, {
      context: setAppContextForTest({
        role: "gm",
        world: "w1",
        documents: storeWith(),
        audio: audioFixture(),
        searchDocuments: searchDocuments as never,
        canCreate: () => true,
        canDelete: () => false,
        openDocument,
        dispatchIntent: (ops) => calls.push(ops),
      }),
    });
    await fireEvent.input(screen.getByTestId("playlists-search"), { target: { value: "tavern" } });
    await waitFor(() =>
      expect(searchDocuments).toHaveBeenCalledWith(
        "tavern",
        expect.objectContaining({ docTypes: ["playlist"] }),
        expect.anything(),
      ),
    );

    await fireEvent.input(screen.getByTestId("playlists-name"), { target: { value: "Tavern Loop" } });
    await fireEvent.click(screen.getByTestId("playlists-create"));
    expect(calls).toHaveLength(1);
    const [ops] = calls as [[{ op: string; doc: WireDocument }]];
    expect(ops[0].op).toBe("create");
    expect(ops[0].doc.doc_type).toBe("playlist");
    expect(ops[0].doc.name).toBe("Tavern Loop");
    expect(openDocument).toHaveBeenCalledWith({ docId: ops[0].doc.id });
  });

  it("hides create when canCreate is false and delete when canDelete is false", async () => {
    const searchDocuments = vi.fn().mockImplementation((_q, _o, onUpdate) => {
      onUpdate([
        {
          document: {
            id: "pl1",
            scope: { kind: "world", world_id: "w1" },
            doc_type: "playlist",
            schema_version: 1,
            name: "Tavern",
            source: null,
            owner: null,
            permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
            embedded: {},
            parent_id: null,
            engine: { tracks: [], mode: "sequential", channel: "music", fadeMs: 0 },
            system: {},
            created_at: 0,
            updated_at: 0,
          },
        },
      ]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    render(AudioPanel, {
      context: setAppContextForTest({
        role: "gm",
        documents: storeWith(),
        audio: audioFixture(),
        searchDocuments: searchDocuments as never,
        canCreate: () => false,
        canDelete: () => false,
      }),
    });
    await fireEvent.input(screen.getByTestId("playlists-search"), { target: { value: "tav" } });
    await screen.findByTestId("playlist-row");
    expect(screen.queryByTestId("playlists-create")).toBeNull();
    expect(screen.queryByTestId("playlist-delete")).toBeNull();
    // The Play and Open affordances are permission-independent for a GM.
    expect(screen.getByTestId("playlist-play")).toBeTruthy();
    expect(screen.getByTestId("playlist-open")).toBeTruthy();
  });
});
