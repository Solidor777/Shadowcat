import { sheetContract, PLAYLIST_DOC_TYPE, type Module } from "@shadowcat/core";
import PlaylistSheet from "./PlaylistSheet.svelte";

export { addTrack, removeTrack, moveTrack, setTrack, defaultTrack } from "./trackOps";

/** The playlist sheet: registers `sheetContract(PLAYLIST_DOC_TYPE)` at priority 0. Edits the
 * playlist's name, playback mode, mixer channel, crossfade duration, and its ordered tracks
 * (asset, per-track name/gain/loop) via a whole-array editor (`trackOps`). */
export const sheetPlaylist: Module = {
  manifest: {
    id: "sheet-playlist",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract(PLAYLIST_DOC_TYPE), cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "sheet-playlist:sheet",
      contract: sheetContract(PLAYLIST_DOC_TYPE),
      component: PlaylistSheet,
      sheet: { priority: 0 },
    });
  },
};
