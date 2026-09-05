import type { CombatApi, CombatAffordances, CombatsView, CombatantView, DocumentStore, WorldRole, WireCombatRollEntry, WireResourceOp, CreateCombatOptions, NewCombatant, NewEvent } from "@shadowcat/core";
import { EMPTY_COMBATS, buildCombatDoc, buildCombatantDoc, newCombatEngine } from "@shadowcat/core";

/** Shape of `CombatEngine`'s `scene_id` field, narrowed for the fake's scene-scoped lookups. */
type SceneScopedShape = {
  /** The combat's bound scene id. */
  scene_id: string;
};
/** Shape of `CombatEngine`'s `active` field, narrowed for the fake's active-combat lookups. */
type CombatActiveShape = {
  /** Whether this is the scene's single running combat. */
  active: boolean;
};
/** Shape of `CombatEngine`'s `order` field, narrowed for the fake's combatant lookups. */
type CombatOrderShape = {
  /** The combat's turn-order sequence of combatant document ids. */
  order: string[];
};
/** Shape of `CombatEngine`'s `turn` field, narrowed for the fake's current-turn lookup. */
type CombatTurnShape = {
  /** The combatant id whose turn is current, or `null` outside an active turn. */
  turn: string | null;
};
/** `fakeCombatApi`'s identity/role options. */
type FakeCombatApiOptions = {
  /** The caller's own user id; defaults to `"u-self"`. */
  selfId?: string;
  /** The caller's world role; defaults to `"gm"`. */
  role?: WorldRole;
};

/** A `CombatApi` test double recording every call, driven by an in-memory `documents` store and
 * a configurable `canAct` result. Every intent method resolves immediately unless
 * `rejectNext` names it, in which case the NEXT call to that method rejects once (then reverts
 * to resolving). */
export interface FakeCombatApi extends CombatApi {
  /** Every call this fake received, in call order, keyed by method name. */
  calls: Record<string, unknown[][]>;
  /** Arms the next call to `method` to reject with `message`.
   * @param method The `CombatApi` method name to arm.
   * @param message The rejection's error message. */
  rejectNext(method: string, message: string): void;
  /** Replaces the affordance set `canAct` returns for every combat id.
   * @param next The affordance overrides to merge over the default all-true set. */
  setCanAct(next: Partial<CombatAffordances>): void;
  /** Replaces the latest resolved `"combat"` frame `resolved`/`resolvedFor` read from.
   * @param view The resolved combats view to serve from then on. */
  setResolved(view: CombatsView): void;
}

/** Builds a `FakeCombatApi` over `documents` (a real `DocumentStore`/`OptimisticClient`), the
 * caller identity, and a role getter.
 * @param documents The document view combat/combatant lookups read from.
 * @param opts Identity + role the affordance defaults are computed from.
 * @returns A recording `CombatApi` double.
 * @example
 * ```
 * declare const documents: DocumentStore;
 * const api = fakeCombatApi(documents, { selfId: "u1", role: "gm" });
 * ```
 */
export function fakeCombatApi(
  documents: DocumentStore,
  opts: FakeCombatApiOptions = {},
): FakeCombatApi {
  const selfId = opts.selfId ?? "u-self";
  const role: WorldRole = opts.role ?? "gm";
  const calls: Record<string, unknown[][]> = {};
  const rejections = new Map<string, string>();
  let canActOverride: Partial<CombatAffordances> = {};
  let resolved: CombatsView = EMPTY_COMBATS;

  /** Appends one call's arguments under `name` in `calls`.
   * @param name The `CombatApi` method name being recorded.
   * @param args The call's argument list.
   * @example
   * ```
   * // private function, closed over `fakeCombatApi`'s own `calls` map — not callable outside
   * // that closure; invoked from every intent method below
   * ```
   */
  function record(name: string, args: unknown[]): void {
    (calls[name] ??= []).push(args);
  }

  /** Consumes and returns a one-shot armed rejection for `name`, or `null` when none is armed.
   * @param name The `CombatApi` method name being checked.
   * @returns A rejected promise carrying the armed message, or `null`.
   * @example
   * ```
   * // private function, closed over `fakeCombatApi`'s own `rejections` map — not callable
   * // outside that closure; invoked from every promise-returning intent method below
   * ```
   */
  function maybeReject(name: string): Promise<void> | null {
    const message = rejections.get(name);
    if (message === undefined) return null;
    rejections.delete(name);
    return Promise.reject(new Error(message));
  }

  const api: FakeCombatApi = {
    calls,
    rejectNext(method, message) {
      rejections.set(method, message);
    },
    setCanAct(next) {
      canActOverride = next;
    },
    setResolved(view) {
      resolved = view;
    },

    get resolved() {
      return resolved;
    },
    resolvedFor(combatantId: string): CombatantView | null {
      for (const c of resolved.combats) {
        const found = c.combatants.find((cc) => cc.id === combatantId);
        if (found) return found;
      }
      return null;
    },
    subscribe: () => () => {},

    combatsFor(sceneId: string) {
      return documents
        .query("combat")
        .filter((d) => (d.engine as SceneScopedShape).scene_id === sceneId)
        .sort((a, b) => {
          const ae = (a.engine as CombatActiveShape).active;
          const be = (b.engine as CombatActiveShape).active;
          if (ae !== be) return ae ? -1 : 1;
          return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
        });
    },
    activeFor(sceneId: string) {
      return api.combatsFor(sceneId).find((d) => (d.engine as CombatActiveShape).active) ?? null;
    },
    combatants(combatId: string) {
      const combat = documents.get(combatId);
      const order = (combat?.engine as CombatOrderShape | undefined)?.order ?? [];
      return order.map((id) => documents.get(id)).filter((d): d is NonNullable<typeof d> => !!d);
    },
    turnOf(combatId: string) {
      const combat = documents.get(combatId);
      const turn = (combat?.engine as CombatTurnShape | undefined)?.turn;
      return turn ? (documents.get(turn) ?? null) : null;
    },
    canAct(): CombatAffordances {
      return {
        start: true, pause: true, end: true, advance: true, rewind: true, sort: true, edit: role === "gm",
        roll: () => true,
        resource: () => true,
        ...canActOverride,
      };
    },

    start(id) {
      record("start", [id]);
      return maybeReject("start") ?? Promise.resolve();
    },
    pause(id) {
      record("pause", [id]);
      return maybeReject("pause") ?? Promise.resolve();
    },
    end(id) {
      record("end", [id]);
      return maybeReject("end") ?? Promise.resolve();
    },
    advance(id) {
      record("advance", [id]);
      return maybeReject("advance") ?? Promise.resolve();
    },
    rewind(id) {
      record("rewind", [id]);
      return maybeReject("rewind") ?? Promise.resolve();
    },
    sort(id) {
      record("sort", [id]);
      return maybeReject("sort") ?? Promise.resolve();
    },
    roll(combatId: string, channel: string, rolls: WireCombatRollEntry[]) {
      record("roll", [combatId, channel, rolls]);
      return maybeReject("roll") ?? Promise.resolve();
    },
    modifyResource(combatId: string, combatantId: string, resource: string, op: WireResourceOp) {
      record("modifyResource", [combatId, combatantId, resource, op]);
      return maybeReject("modifyResource") ?? Promise.resolve();
    },

    createCombat(sceneId: string, createOpts: CreateCombatOptions = {}) {
      record("createCombat", [sceneId, createOpts]);
      const message = rejections.get("createCombat");
      if (message !== undefined) {
        rejections.delete("createCombat");
        throw new Error(message);
      }
      const doc = buildCombatDoc("w1", newCombatEngine(sceneId), createOpts.id);
      doc.name = createOpts.name ?? null;
      documents.applyCommand({ seq: 1, world_id: "w1", author: selfId, ts: 0, ops: [{ op: "create", doc }] });
      return doc.id;
    },
    deleteCombat(id) {
      record("deleteCombat", [id]);
    },
    addCombatants(combatId: string, entries: NewCombatant[]) {
      record("addCombatants", [combatId, entries]);
      return entries.map((_, i) => `new-${i}`);
    },
    addEvent(combatId: string, ev: NewEvent) {
      record("addEvent", [combatId, ev]);
      return "new-event";
    },
    removeCombatant(combatId: string, combatantId: string) {
      record("removeCombatant", [combatId, combatantId]);
    },
    setHidden(combatantId: string, hidden: boolean) {
      record("setHidden", [combatantId, hidden]);
    },
    reorder(combatId: string, order: string[]) {
      record("reorder", [combatId, order]);
    },
    setInitiative(combatantId: string, initiative: number | null, tiebreak?: number) {
      record("setInitiative", [combatantId, initiative, tiebreak]);
    },
  };
  return api;
}

export { buildCombatDoc, buildCombatantDoc, newCombatEngine };
