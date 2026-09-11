import { sheetContract, NOTE_DOC_TYPE, type Module } from "@shadowcat/core";
import NoteSheet from "./NoteSheet.svelte";

/** Note sheet: registers `sheetContract(NOTE_DOC_TYPE)` at priority 0 — the reference
 * (and only, on this branch) sheet for the `note` doc_type. Body rendering, visibility,
 * the draft-base OCC edit flow, and the parent/children tree are documented on
 * `NoteSheet.svelte` itself. */
export const sheetNote: Module = {
  manifest: {
    id: "sheet-note",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract(NOTE_DOC_TYPE), cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute(
      { id: "sheet-note:sheet", contract: sheetContract(NOTE_DOC_TYPE), component: NoteSheet, sheet: { priority: 0 } },
    );
  },
};
