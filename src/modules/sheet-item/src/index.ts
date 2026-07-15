import { sheetContract, ITEM_DOC_TYPE, type Module } from "@shadowcat/core";
import ItemSheet from "./ItemSheet.svelte";

/** Item sheet for the client-only `item` doc_type. Registers at priority 0 for the
 * `item` doc_type. Dice-notation string values get a roll-to-chat affordance. */
export const sheetItem: Module = {
  manifest: {
    id: "sheet-item",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract(ITEM_DOC_TYPE), cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute(
      { id: "sheet-item:sheet", contract: sheetContract(ITEM_DOC_TYPE), component: ItemSheet, sheet: { priority: 0 } },
    );
  },
};
