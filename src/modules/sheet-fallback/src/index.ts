import { SHEET_FALLBACK_CONTRACT, type Module } from "@shadowcat/core";
import FallbackSheet from "./FallbackSheet.svelte";

/** The always-registered generic document sheet. Registers under the fallback
 * contract at `-Infinity` priority so a doc_type-specific provider always wins, but every
 * document can still open. Replaceable — a game-system module can raise the bar. */
export const sheetFallback: Module = {
  manifest: {
    id: "sheet-fallback",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: SHEET_FALLBACK_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute({
      id: "sheet-fallback:sheet",
      contract: SHEET_FALLBACK_CONTRACT,
      component: FallbackSheet,
      sheet: { priority: -Infinity },
    });
  },
};
