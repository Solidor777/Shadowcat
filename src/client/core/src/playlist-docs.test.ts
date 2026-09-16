// @vitest-environment node
// Exercises plain document builders: no component render and no DOM API use.
import { describe, test, expect } from "vitest";
import { buildPlaylistDoc, PLAYLIST_DOC_TYPE } from "./playlist-docs";
import { AUTHOR_CAPS } from "./scene-docs";
import type { PlaylistEngine } from "@shadowcat/types";

const engine: PlaylistEngine = {
  tracks: [{ asset: "a1", name: null, gain: 0.8, loop: true }],
  mode: "loop_all",
  channel: "music",
  fadeMs: 1500,
};

describe("buildPlaylistDoc", () => {
  test("builds a standalone, observer-default playlist document", () => {
    const doc = buildPlaylistDoc("w1", "Tavern", engine);
    expect(doc.doc_type).toBe(PLAYLIST_DOC_TYPE);
    expect(doc.name).toBe("Tavern");
    expect(doc.parent_id).toBeNull();
    expect(doc.engine).toEqual(engine);
    expect(doc.system).toEqual({});
    expect(doc.permissions.default).toBe("observer");
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  });

  test("uses the explicit id when given", () => {
    const doc = buildPlaylistDoc("w1", "Tavern", engine, { id: "p1" });
    expect(doc.id).toBe("p1");
  });

  test("grants the author Owner plus AUTHOR_CAPS when opts.owner is given", () => {
    const doc = buildPlaylistDoc("w1", "Tavern", engine, { owner: "gm-1" });
    expect(doc.permissions.users).toEqual({ "gm-1": "owner" });
    for (const cap of AUTHOR_CAPS) {
      expect(doc.permissions.capabilities.by_role.owner).toContain(cap);
    }
  });
});
