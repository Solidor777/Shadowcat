// Pure helpers over combat documents + the resolved "combat" channel — no reactive reads, no
// intent dispatch. Everything here is unit-tested directly; the panel/header/row components call
// into it rather than re-deriving the same joins inline.
import type {
  WireDocument,
  CombatsView,
  CombatantView,
  ResolvedResourceView,
  CombatantEngine,
  WorldRole,
} from "@shadowcat/core";

/** One rendered tracker row: a combatant document joined with its resolved numbers. */
export interface Row {
  /** The combatant document. */
  doc: WireDocument;
  /** `"actor"` for a token/actor combatant, `"event"` for a one-shot event row. */
  kind: "actor" | "event";
  /** The combatant's resolved numbers from the latest `"combat"` frame, or `null` when this
   * combatant is absent from it (no frame has arrived yet, or the recipient cannot read it —
   * though an unreadable combatant is never in `combatants` to begin with). */
  view: CombatantView | null;
  /** Display name; `null` for a redacted or unnamed document. */
  name: string | null;
  /** Art source: the combatant's token and/or actor id, for face-asset resolution. */
  art: {
    /** The combatant's own token id, when it names one. */
    tokenId?: string;
    /** The combatant's linked actor id, when it names one. */
    actorId?: string;
  };
}

/** Joins `combat`'s combatant documents with their resolved `"combat"`-channel numbers, in the
 * given input order (the caller — `CombatApi.combatants` — already resolves `engine.order`).
 * @param combatants The combatant documents, in the order to render them.
 * @param resolved The latest `"combat"` derived-channel frame.
 * @returns One `Row` per combatant document.
 * @example
 * ```
 * import type { CombatApi } from "@shadowcat/core";
 *
 * declare const combatApi: CombatApi;
 * rowsFor(combatApi.combatants("c1"), combatApi.resolved);
 * ```
 */
export function rowsFor(combatants: WireDocument[], resolved: CombatsView): Row[] {
  const views = new Map<string, CombatantView>();
  for (const c of resolved.combats) {
    for (const cc of c.combatants) views.set(cc.id, cc);
  }
  return combatants.map((doc) => {
    const engine = doc.engine as CombatantEngine;
    const kind = engine.kind.type;
    const art: Row["art"] = {};
    if (kind === "actor") {
      if (engine.kind.token_id) art.tokenId = engine.kind.token_id;
      if (engine.kind.actor_id) art.actorId = engine.kind.actor_id;
    }
    return { doc, kind, view: views.get(doc.id) ?? null, name: doc.name, art };
  });
}

/** Moves the element at `from` to `to`, shifting the elements between. Same-multiset invariant:
 * the result is always a permutation of `order`.
 * @param order The current order.
 * @param from The index to move.
 * @param to The destination index.
 * @returns A new array with the element relocated; the SAME array reference is never returned
 * even when `from === to` (a no-op still produces a fresh, equal array).
 * @throws {RangeError} When `from` or `to` is out of `order`'s bounds.
 * @example
 * ```
 * moveInOrder(["a", "b", "c"], 0, 2); // ["b", "c", "a"]
 * ```
 */
export function moveInOrder(order: string[], from: number, to: number): string[] {
  if (from < 0 || from >= order.length || to < 0 || to >= order.length) {
    throw new RangeError(`moveInOrder: index out of range (from=${from}, to=${to}, length=${order.length})`);
  }
  const next = order.slice();
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}

/** The "Roll all" target set: actor combatants without an initiative yet, whose roll the caller
 * may make. A GM may roll every such row; a player may roll only their own.
 * @param rows The panel's current rows.
 * @param role The caller's world role.
 * @param selfId The caller's own user id.
 * @returns The target combatant ids, in row order.
 * @example
 * ```
 * declare const rows: Row[];
 * rollTargets(rows, "player", "u1");
 * ```
 */
export function rollTargets(rows: Row[], role: WorldRole, selfId: string): string[] {
  return rows
    .filter((r) => r.kind === "actor" && (r.doc.engine as CombatantEngine).initiative === null)
    .filter((r) => role === "gm" || r.doc.owner === selfId)
    .map((r) => r.doc.id);
}

/** Renders one resolved resource view as tracker cell text.
 * @param view The resolved resource view, or `undefined` when this recipient may not see it
 * (a `resources: null` cell, or a registry key absent from the resolved frame).
 * @returns `"12 / 12"` for a tracked resource, its current value alone for a mirror, `"⚠"` on an
 * evaluation error, and `"—"` when `view` is `undefined`.
 * @example
 * ```
 * formatResource({ binding: "tracked", current: 2, max: 2, error: null }); // "2 / 2"
 * ```
 */
export function formatResource(view: ResolvedResourceView | undefined): string {
  if (!view) return "—";
  if (view.error) return "⚠";
  if (view.binding === "tracked") return `${view.current} / ${view.max}`;
  return `${view.current}`;
}
