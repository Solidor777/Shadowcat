// #region manifest
import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import InitiativePanel from "./InitiativePanel.svelte";

/** One tracked combatant row: the actor's doc id, display name, and rolled score. */
export interface Entry {
  actorId: string;
  name: string;
  initiative: number;
}

/**
 * Rolls a d20 initiative score.
 * @example
 * ```ts
 * import { rollInitiative } from "shadowcat-example-initiative-tracker";
 *
 * const score = rollInitiative(() => Math.random()); // 1..=20
 * ```
 */
export function rollInitiative(rng: () => number): number {
  return Math.floor(rng() * 20) + 1;
}

/**
 * Turn order: initiative descending; ties break by name ascending so the order
 * is stable across re-renders.
 * @example
 * ```ts
 * import { sortEntries } from "shadowcat-example-initiative-tracker";
 *
 * const ordered = sortEntries([{ actorId: "a", name: "MOCK_ACTOR_A", initiative: 3 }]);
 * ```
 */
export function sortEntries(entries: Entry[]): Entry[] {
  return [...entries].sort((a, b) => b.initiative - a.initiative || a.name.localeCompare(b.name));
}

/** Tutorial module: contributes one GM panel that rolls + tracks initiative and
 * writes each roll onto the actor's opaque `system` band. */
const initiativeTracker: Module = {
  manifest: {
    id: "example-initiative-tracker",
    version: "0.1.0",
    dependencies: {},
    requires: [PANEL_CONTRACT],
    provides: [],
    engines: { shadowcat: "^0.1.0" },
  },
  // #endregion manifest
  // #region register
  register(ctx) {
    ctx.contributions.contribute({
      id: "example-initiative-tracker:panel",
      contract: PANEL_CONTRACT,
      component: InitiativePanel,
      // labelKey falls back to its literal value for keys absent from the host
      // catalog — community modules have no i18n registration seam yet.
      panel: { icon: "⚔️", labelKey: "Initiative", gmOnly: true },
    });
  },
  // #endregion register
};

export default initiativeTracker;
