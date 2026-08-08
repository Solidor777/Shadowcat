import { sheetContract, type Module } from "@shadowcat/core";
import ActorSheet from "./ActorSheet.svelte";

/** Actor sheet: engine-known fields + `system` tree editor + embedded-items
 * inventory. Registers for the `actor` doc_type at priority 0 — a game-system module
 * raises the bar with a higher priority provider. */
export const sheetActor: Module = {
  manifest: {
    id: "sheet-actor",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract("actor"), cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute(
      { id: "sheet-actor:sheet", contract: sheetContract("actor"), component: ActorSheet, sheet: { priority: 0 } },
    );
  },
};
