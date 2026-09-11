import { sheetContract, TABLE_DOC_TYPE, type Module } from "@shadowcat/core";
import TableSheet from "./TableSheet.svelte";

export { addRow, removeRow, moveRow, setRow, defaultEntry, nextFreeRange } from "./rowOps";

/** Rollable-table sheet: registers `sheetContract(TABLE_DOC_TYPE)` at priority 0 — the
 * reference (and only, on this branch) sheet for the `table` doc_type. Draw/rows/entry
 * editing is documented on `TableSheet.svelte` itself. */
export const sheetTable: Module = {
  manifest: {
    id: "sheet-table",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract(TABLE_DOC_TYPE), cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute(
      { id: "sheet-table:sheet", contract: sheetContract(TABLE_DOC_TYPE), component: TableSheet, sheet: { priority: 0 } },
    );
  },
};
