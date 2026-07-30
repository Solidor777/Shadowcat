// #region manifest
import { sheetContract, type Module } from "@shadowcat/core";
import CharacterSheet from "./CharacterSheet.svelte";

export { abilityMod, evalFormula } from "./rules";

/** Tutorial system: replaces the generic actor sheet with a minimal d20-style
 * character sheet (attributes + formula-derived values on the `system` band). */
const systemMinimal: Module = {
  manifest: {
    id: "example-system-minimal",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: sheetContract("actor"), cardinality: "multi" }],
    engines: { shadowcat: "^0.1.0" },
  },
  // #endregion manifest
  // #region sheet-registration
  register(ctx) {
    ctx.contributions.contribute({
      id: "example-system-minimal:actor-sheet",
      contract: sheetContract("actor"),
      component: CharacterSheet,
      // Priority 1 outranks the built-in generic actor sheet (priority 0):
      // a game system claims the doc_type by registering higher.
      sheet: { priority: 1 },
    });
  },
  // #endregion sheet-registration
};

export default systemMinimal;
