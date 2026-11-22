// Pure helpers over combat documents + the resolved "combat" channel — no reactive reads, no
// intent dispatch. Everything here is unit-tested directly; the panel/header/row components call
// into it rather than re-deriving the same joins inline.
import type {
  WireDocument,
  CombatsView,
  CombatantView,
  ResolvedResourceView,
  CombatantEngine,
  CombatantGroupEngine,
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
  /** The combatant's group (id, name, accent color), or `null` when ungrouped or when the id
   * names no readable group document. */
  group: {
    /** The `combatant-group` document id. */
    id: string;
    /** The group's display name. */
    name: string;
    /** The group's css `#rrggbb` accent, or `null`. */
    color: string | null;
  } | null;
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
 * @param groups The combat's `combatant-group` documents, joined onto each row's `group`.
 * @returns One `Row` per combatant document.
 * @example
 * ```
 * import type { CombatApi } from "@shadowcat/core";
 *
 * declare const combatApi: CombatApi;
 * rowsFor(combatApi.combatants("c1"), combatApi.resolved, combatApi.groups("c1"));
 * ```
 */
export function rowsFor(combatants: WireDocument[], resolved: CombatsView, groups: WireDocument[] = []): Row[] {
  const groupById = new Map<string, WireDocument>();
  for (const g of groups) groupById.set(g.id, g);
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
    const gdoc = engine.group ? groupById.get(engine.group) : undefined;
    const gengine = gdoc?.engine as CombatantGroupEngine | undefined;
    const group = gdoc && gengine ? { id: gdoc.id, name: gengine.name, color: gengine.color } : null;
    return { doc, kind, view: views.get(doc.id) ?? null, name: doc.name, group, art };
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
 * may make. A GM may roll every such row; a player may roll only their own. A grouped combatant
 * collapses to ONE target, its group's id, placed at the group's first row: the server rolls the
 * group once and stamps every member. A player gets a group target only when they own every
 * actor member, mirroring the server's all-members ownership check; otherwise the group is
 * skipped.
 * @param rows The panel's current rows.
 * @param role The caller's world role.
 * @param selfId The caller's own user id.
 * @returns The target combatant and group ids, in row order.
 * @example
 * ```
 * declare const rows: Row[];
 * rollTargets(rows, "player", "u1");
 * ```
 */
export function rollTargets(rows: Row[], role: WorldRole, selfId: string): string[] {
  const out: string[] = [];
  const seenGroups = new Set<string>();
  for (const r of rows) {
    if (r.kind !== "actor") continue;
    const engine = r.doc.engine as CombatantEngine;
    if (engine.group) {
      if (seenGroups.has(engine.group)) continue;
      seenGroups.add(engine.group);
      const members = rows.filter((m) => (m.doc.engine as CombatantEngine).group === engine.group);
      const pending = members.some((m) => (m.doc.engine as CombatantEngine).initiative === null);
      const allowed = role === "gm" || members.every((m) => m.doc.owner === selfId);
      if (pending && allowed) out.push(engine.group);
      continue;
    }
    if (engine.initiative === null && (role === "gm" || r.doc.owner === selfId)) out.push(r.doc.id);
  }
  return out;
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
