// #region manifest
import { PANEL_CONTRACT, type Module } from "@shadowcat/core";
import InitiativePanel from "./InitiativePanel.svelte";

/** One tracked combatant row: the actor's doc id, display name, and rolled score. */
export interface Entry {
  /** The rolled actor's document id — `sortEntries`' tie-break reads `name`, not this field. */
  actorId: string;
  /** Display name captured at roll time (`InitiativePanel.roll`); not re-read from the actor
   * document afterward, so a later rename does not retroactively relabel the tracked entry. */
  name: string;
  /** The `rollInitiative` result for this actor; `sortEntries`' primary (descending) sort key. */
  initiative: number;
}

/**
 * Rolls a d20 initiative score. `rng` is injected (rather than calling
 * `Math.random()` directly) so callers can supply a deterministic source for
 * tests. CONTRACT: `rng` must return a value in `[0, 1)` — `Math.random()`'s
 * own documented range. Given that contract, the result is `1..=20` inclusive;
 * an `rng` that could return exactly `1.0` would break the upper bound (yielding
 * `21`), so callers must not pass one.
 * @param rng - Returns a pseudo-random number in `[0, 1)` (e.g. `Math.random`).
 * @returns An integer in `1..=20` inclusive, given `rng`'s contract holds.
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
 * is stable across re-renders — a re-roll can repeat an initiative value, and
 * name is the tie-break that keeps the on-screen ordering from jumping when it
 * does. Returns a NEW array; `entries` itself is not mutated (unlike calling
 * `Array.prototype.sort` on it directly).
 * @param entries - The combatants to order; not mutated.
 * @returns A new array containing the same entries, sorted.
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
    ctx.i18n.addMessages("en", { "example-initiative-tracker.panelLabel": "Initiative" });
    ctx.contributions.contribute({
      id: "example-initiative-tracker:panel",
      contract: PANEL_CONTRACT,
      component: InitiativePanel,
      // labelKey resolves against the host's i18n catalog via `ctx.i18n.addMessages`, registered
      // just above — a key with no registered message falls back to its literal string (`I18n.t`).
      panel: { icon: "⚔️", labelKey: "example-initiative-tracker.panelLabel", gmOnly: true },
    });
  },
  // #endregion register
};

export default initiativeTracker;
