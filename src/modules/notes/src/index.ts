import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import NotesPanel from "./NotesPanel.svelte";

/** Note tree + search panel. Requires the panel-manager's contract; contributes NotesPanel
 * launcher-closed by default. Not `gmOnly` — players read shared notes and author private
 * ones when granted `core:create`. */
export const notes: Module = {
  manifest: {
    id: "notes",
    version: "0.1.0",
    dependencies: { "core-ui": "^0.1.0" },
    requires: [PANEL_CONTRACT],
    provides: [],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "notes:panel",
      contract: PANEL_CONTRACT,
      order: 3,
      component: NotesPanel,
      panel: { icon: "📓", labelKey: "notes.tab" },
    });
  },
};
