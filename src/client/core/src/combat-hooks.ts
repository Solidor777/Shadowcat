// The combat clock's hook surface: pure derivation of typed events from an applied command's
// delta over the combat document it touched and the turn records its `combat-history` write
// appended, plus the emitter that chains them onto a HookBus in seq order. No formula
// evaluation, no server round-trip and no re-derivation of the server's turn walk here -- every
// event is read off the same authoritative documents WorldSession already applies.
import type { HookBus, CoreHooks } from "./hooks";
import type { Logger } from "./logger";
import type { ReadableDocuments } from "./store";
import { getPointer } from "./store";
import type { WireDocument, WireCommand, WireOperation } from "./wire";
import type { CombatEngine, CombatantEngine, CombatHistoryEngine, TurnRecord } from "./scene-docs";
import { COMBAT_HISTORY_DOC_TYPE } from "./scene-docs";

/** Payload common to every `combat:*` hook: which combat the event concerns. */
interface CombatEventBase {
  /** The combat document's id. */
  combatId: string;
}

/** Payload for `combat:start`. */
export interface CombatStartEvent extends CombatEventBase {
  /** The combat's scene. */
  sceneId: string;
  /** The round the clock is now on. */
  round: number;
  /** `true` when the combat's turn field is already non-null at the moment the clock starts
   * (a paused-then-restarted combat); `false` on a fresh start from `round: 0`. */
  resumed: boolean;
}

/** Payload for `combat:end`. */
export interface CombatEndEvent extends CombatEventBase {
  /** The combat's scene. */
  sceneId: string;
  /** The round the clock was on when it stopped. */
  round: number;
  /** `"paused"` when the combat document still exists; `"ended"` when it was deleted. */
  reason: "paused" | "ended";
}

/** Payload for `combat:round-start`/`combat:round-end`. */
export interface CombatRoundEvent extends CombatEventBase {
  /** The round this event concerns. */
  round: number;
}

/** Payload for `combat:turn-start`/`combat:turn-end`. */
export interface CombatTurnEvent extends CombatEventBase {
  /** The round the turn falls in. */
  round: number;
  /** The combatant document id whose turn this is. */
  combatantId: string;
  /** The combatant's kind, or `null` when the store cannot resolve `combatantId` (a hidden
   * combatant). */
  kind: "actor" | "event" | null;
}

/** Payload for `combat:rewind`. */
export interface CombatRewindEvent extends CombatEventBase {
  /** The round the clock rewound to. */
  round: number;
  /** The turn the clock rewound to, or `null`. */
  turn: string | null;
}

/** Payload common to `combat:effect-tick`/`combat:effect-expired`. */
interface CombatEffectEventBase extends CombatEventBase {
  /** The round the attributed combat is currently on. */
  round: number;
  /** The document carrying the embedded effect. */
  hostId: string;
  /** JSON-pointer prefix of the effect within `hostId` (its own embedded root). */
  path: string;
  /** The embedded effect's own id, or `null` when the host cannot be resolved. */
  effectId: string | null;
}

/** Payload for `combat:effect-tick`. */
export interface CombatEffectTickEvent extends CombatEffectEventBase {
  /** The effect's new remaining-duration count. */
  remaining: number;
}

/** Payload for `combat:effect-expired`. */
export type CombatEffectExpiredEvent = CombatEffectEventBase;

declare module "./hooks" {
  /** Declaration-merged with `./hooks`'s own (empty) `CoreHooks` to add the nine `combat:*`
   * entries below. */
  interface CoreHooks {
    /** A combat began running its clock (`combat_start` accepted, or resumed off a paused
     * turn). */
    "combat:start": CombatStartEvent;
    /** A combat stopped running its clock, either paused (still exists) or ended (deleted). */
    "combat:end": CombatEndEvent;
    /** A round completed; never fires for round 0 (creation is not a round boundary). */
    "combat:round-end": CombatRoundEvent;
    /** A new round began. */
    "combat:round-start": CombatRoundEvent;
    /** A combatant's (or intermediate event's) turn began. */
    "combat:turn-start": CombatTurnEvent;
    /** A combatant's (or intermediate event's) turn ended. */
    "combat:turn-end": CombatTurnEvent;
    /** The GM rewound the clock to an earlier turn record. */
    "combat:rewind": CombatRewindEvent;
    /** An embedded effect's remaining-duration counter decreased (or was newly set). */
    "combat:effect-tick": CombatEffectTickEvent;
    /** An embedded effect's `active` flag flipped from `true` to `false`. */
    "combat:effect-expired": CombatEffectExpiredEvent;
  }
}

/** Semver of the nine `combat:*` hook payload contracts declared above. */
export const COMBAT_HOOK_VERSION = "1.0.0";

/** One member of the `CombatHookEvent` union: a hook name paired with its declared payload. */
interface CombatHookEventFor<K extends keyof CoreHooks> {
  /** The hook name this event was derived for. */
  name: K;
  /** The payload declared for `name` on `CoreHooks`. */
  payload: CoreHooks[K];
}

/** One derived combat hook event, tagged by name with its matching payload type. */
export type CombatHookEvent = {
  [K in keyof CoreHooks]: CombatHookEventFor<K>;
}[keyof CoreHooks];

/** Declares all nine `combat:*` hooks on `hooks` as `"info"` kind at {@link COMBAT_HOOK_VERSION}.
 * Idempotent to call more than once at the same version (a second `WorldSession` construction
 * in the same process, e.g. under test).
 * @param hooks The bus to declare against.
 * @example
 * ```ts
 * import { HookBus, silentLogger } from "@shadowcat/core";
 * import { defineCombatHooks } from "@shadowcat/core";
 *
 * const hooks = new HookBus(silentLogger);
 * defineCombatHooks(hooks);
 * ```
 */
export function defineCombatHooks(hooks: HookBus): void {
  const names: (keyof CoreHooks)[] = [
    "combat:start",
    "combat:end",
    "combat:round-start",
    "combat:round-end",
    "combat:turn-start",
    "combat:turn-end",
    "combat:rewind",
    "combat:effect-tick",
    "combat:effect-expired",
  ];
  for (const name of names) {
    hooks.defineHook(name, { version: COMBAT_HOOK_VERSION, kind: "info" });
  }
}

/** Cheap pre-scan: does `cmd` touch a `combat`/`combatant` document, or an embedded effect's
 * `duration/remaining`/`active` field? `WorldSession.onCommand` uses this to skip building a
 * pre-image map (and calling {@link deriveCombatHookEvents}) for ordinary token/scene commands.
 * @param cmd The command to scan.
 * @param store The document view to resolve an `update` op's pre-touch doc type against.
 * @returns `true` if the command could produce a combat hook event.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import { commandTouchesCombat } from "@shadowcat/core";
 * import type { WireCommand } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * commandTouchesCombat(cmd, store);
 * ```
 */
export function commandTouchesCombat(cmd: WireCommand, store: ReadableDocuments): boolean {
  for (const op of cmd.ops) {
    if (op.op === "create" || op.op === "delete") {
      if (op.doc.doc_type === "combat" || op.doc.doc_type === "combatant") return true;
    } else if (op.op === "move") {
      const doc = store.get(op.doc_id);
      if (doc && (doc.doc_type === "combat" || doc.doc_type === "combatant")) return true;
    } else {
      const doc = store.get(op.doc_id);
      if (doc && (doc.doc_type === "combat" || doc.doc_type === "combatant")) return true;
      if (op.changes.some((c) => EFFECT_PATH.test(c.path))) return true;
    }
  }
  return false;
}

/** Matches an embedded effect's `duration/remaining` or `active` field at any embedded-child
 * nesting depth; capture group 1 is the embedded prefix (the effect's own JSON-pointer root),
 * group 2 the changed leaf. */
const EFFECT_PATH = /^((?:\/embedded\/[^/]+\/\d+)+)\/engine\/(duration\/remaining|active)$/;

/** One combat document's pre/post engine body as touched by one command; `b`/`a` are `undefined`
 * on the create/delete side respectively. */
interface CombatTouch {
  /** The touched combat document's id. */
  id: string;
  /** The combat's engine body before the command; `undefined` on a `create`. */
  b: CombatEngine | undefined;
  /** The combat's engine body after the command; `undefined` on a `delete`. */
  a: CombatEngine | undefined;
}

/** The combatant kind a document's `engine.kind.type` names, or `null` for a non-combatant document.
 * @param doc The document to inspect, or `undefined` for a doc that no longer exists.
 * @returns `"actor"` | `"event"` | `null`.
 * @example
 * ```ts
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const combatantDoc: WireDocument;
 * combatantKindOf(combatantDoc); // "actor" | "event"
 * combatantKindOf(undefined); // null
 * ```
 */
function combatantKindOf(doc: WireDocument | undefined): "actor" | "event" | null {
  const engine = doc?.engine as CombatantEngine | undefined;
  return engine?.kind.type ?? null;
}

/** Walks one command's ops for every touched `combat` document's before/after engine pair.
 * @param cmd The applied command whose ops to scan.
 * @param before Looks up a document's pre-image by id (`undefined` for a `create`).
 * @param after The post-command document view (a `DocumentStore`-shaped reader).
 * @returns One `CombatTouch` per touched combat, keyed by its document id.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import type { WireCommand } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * declare const combatId: string;
 * const touches = collectCombatTouches(cmd, (id) => store.get(id), store);
 * touches.get(combatId)?.a?.active; // true | undefined
 * ```
 */
function collectCombatTouches(
  cmd: WireCommand,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): Map<string, CombatTouch> {
  const touches = new Map<string, CombatTouch>();
  for (const op of cmd.ops) {
    if (op.op === "create" && op.doc.doc_type === "combat") {
      touches.set(op.doc.id, { id: op.doc.id, b: undefined, a: op.doc.engine as CombatEngine });
    } else if (op.op === "delete" && op.doc.doc_type === "combat") {
      touches.set(op.doc.id, { id: op.doc.id, b: op.doc.engine as CombatEngine, a: undefined });
    } else if (op.op === "update") {
      const b = before(op.doc_id);
      if (b?.doc_type !== "combat") continue;
      const a = after.get(op.doc_id);
      touches.set(op.doc_id, { id: op.doc_id, b: b.engine as CombatEngine, a: a?.engine as CombatEngine | undefined });
    }
  }
  return touches;
}

/** One combat's `combat-history` document as touched by one command: the engine body before
 * the command (`undefined` when the command created the document) and after it. A recipient
 * only ever sees this for a history op its own filtered stream delivered — the document is
 * GM-only egress, so a player's derivation never holds one. */
interface HistoryTouch {
  /** The history engine before the command; `undefined` when the command created it, and
   * possibly holding no record at `cursor` (a client-created empty log). */
  b: CombatHistoryEngine | undefined;
  /** The history engine after the command. */
  a: CombatHistoryEngine;
}

/** One turn boundary the clock crossed within a command: the round entered and the combatant
 * whose turn began — the `round`/`turn` half of a `TurnRecord`. */
interface TurnStep {
  /** The round the turn falls in. */
  round: number;
  /** The combatant document id whose turn began. */
  turn: string;
}

/** Finds `combatId`'s `combat-history` op in `cmd`, when this recipient received one.
 * @param cmd The applied command whose ops to scan.
 * @param combatId The combat the history document must be parented to.
 * @param before Looks up a document's pre-image by id (`undefined` for a `create`).
 * @param after The post-command document view (a `DocumentStore`-shaped reader).
 * @returns The history engine pair, or `undefined` when the command carried no visible history op.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import type { WireCommand } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * declare const combatId: string;
 * collectHistoryTouch(cmd, combatId, (id) => store.get(id), store)?.a.cursor; // number | undefined
 * ```
 */
function collectHistoryTouch(
  cmd: WireCommand,
  combatId: string,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): HistoryTouch | undefined {
  for (const op of cmd.ops) {
    if (op.op === "create") {
      if (op.doc.doc_type === COMBAT_HISTORY_DOC_TYPE && op.doc.parent_id === combatId) {
        return { b: undefined, a: op.doc.engine as CombatHistoryEngine };
      }
    } else if (op.op === "update") {
      const b = before(op.doc_id);
      if (b?.doc_type !== COMBAT_HISTORY_DOC_TYPE || b.parent_id !== combatId) continue;
      const a = after.get(op.doc_id);
      if (!a) continue;
      return { b: b.engine as CombatHistoryEngine, a: a.engine as CombatHistoryEngine };
    }
  }
  return undefined;
}

/** The turn records a command's history write placed between the pre-command current record
 * and the new cursor — every boundary the server's walk crossed, auto-resolved entries
 * included. The server appends past the current record after truncating any redo tail and may
 * evict the oldest records, so the pre-command current record is located by its `(round, turn)`
 * identity scanning DOWN from its old index (eviction only ever shifts it left, and no record
 * the same command appends can share its identity — the clock always moves off the current
 * turn); a fast-forward, which moves `cursor` over records already present, crosses exactly
 * those.
 * @param h The history engine pair for one combat.
 * @returns The crossed records, oldest first; `null` when the pre-command current record cannot
 * be located in the post-image (the caller falls back to the combat document's own endpoints).
 * @example
 * ```ts
 * import type { CombatHistoryEngine } from "@shadowcat/core";
 *
 * declare const b: CombatHistoryEngine;
 * declare const a: CombatHistoryEngine;
 * crossedRecords({ b, a })?.map((r) => r.turn); // the combatant ids entered, in walk order
 * ```
 */
function crossedRecords(h: HistoryTouch): TurnRecord[] | null {
  const upTo = h.a.records.slice(0, h.a.cursor + 1);
  const oldCurrent = h.b?.records[h.b.cursor];
  if (!h.b || !oldCurrent) return upTo;
  for (let i = Math.min(h.b.cursor, upTo.length - 1); i >= 0; i--) {
    const r = upTo[i];
    if (r.round === oldCurrent.round && r.turn === oldCurrent.turn) return upTo.slice(i + 1);
  }
  return null;
}

/** The ordered turn boundaries one command crossed for one combat: the server-recorded walk
 * when the recipient received the combat's `combat-history` write (GM), else the one boundary
 * the combat document's own `turn`/`round` endpoints evidence — a moved `turn`, or the same
 * `turn` in a later `round` (a full lap of a one-entry or all-auto-resolving order), each of
 * which is a turn boundary by the clock's own definition rather than a re-derivation of the
 * server's auto-resolve walk. A recipient without the history write therefore observes the
 * endpoints only; intermediate auto-resolved turns are visible to whoever the record reaches.
 * @param touch The combat's before/after engine pair.
 * @param history The combat's history pair, when the command carried a visible one.
 * @returns The boundaries in the order the clock crossed them; empty when the turn did not move.
 * @example
 * ```ts
 * import type { CombatEngine } from "@shadowcat/core";
 *
 * declare const touch: { id: string; b: CombatEngine | undefined; a: CombatEngine | undefined };
 * turnWalk(touch, undefined).map((s) => s.turn); // e.g. ["combatant-b"]
 * ```
 */
function turnWalk(touch: CombatTouch, history: HistoryTouch | undefined): TurnStep[] {
  const { b, a } = touch;
  if (!a || a.turn == null) return [];
  if (history) {
    const crossed = crossedRecords(history);
    if (crossed && crossed.length > 0) return crossed.map((r) => ({ round: r.round, turn: r.turn }));
  }
  const moved = !b || a.turn !== b.turn || a.round !== b.round;
  return moved ? [{ round: a.round, turn: a.turn }] : [];
}

/** One `combat:turn-start`/`combat:turn-end` event. `kind` resolves against the post-image
 * first, then the pre-image: an exhausted `Event` deleted by the same command is only findable
 * in the delete op's pre-image.
 * @param combatId The combat the turn belongs to.
 * @param name Which of the two turn hooks.
 * @param round The round the turn falls in.
 * @param turn The combatant document id.
 * @param before Looks up a document's pre-image by id.
 * @param after The post-command document view.
 * @returns The event, ready to push.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * turnEvent("combat-1", "combat:turn-start", 1, "combatant-a", (id) => store.get(id), store).name; // "combat:turn-start"
 * ```
 */
function turnEvent(
  combatId: string,
  name: "combat:turn-start" | "combat:turn-end",
  round: number,
  turn: string,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): CombatHookEvent {
  return {
    name,
    payload: { combatId, round, combatantId: turn, kind: combatantKindOf(after.get(turn) ?? before(turn)) },
  };
}

/** Pushes the `combat:round-end`/`combat:round-start` pairs for every round boundary between
 * `from` and `to`; round 0 (creation) never ends.
 * @param events The event list to append to.
 * @param combatId The combat the rounds belong to.
 * @param from The round the clock is leaving.
 * @param to The round the clock lands on.
 * @example
 * ```ts
 * import type { CombatHookEvent } from "@shadowcat/core";
 *
 * const events: CombatHookEvent[] = [];
 * pushRounds(events, "combat-1", 1, 2);
 * events.map((e) => e.name); // ["combat:round-end", "combat:round-start"]
 * ```
 */
function pushRounds(events: CombatHookEvent[], combatId: string, from: number, to: number): void {
  for (let r = from; r < to; r++) {
    if (r > 0) events.push({ name: "combat:round-end", payload: { combatId, round: r } });
    events.push({ name: "combat:round-start", payload: { combatId, round: r + 1 } });
  }
}

/** Derives one combat's `combat:*` hook events from its before/after engine pair and, when the
 * command carried it, the combat's history write.
 * @param touch The combat's before/after engine pair.
 * @param cmd The applied command touch's ops came from.
 * @param before Looks up a document's pre-image by id.
 * @param after The post-command document view (a `DocumentStore`-shaped reader).
 * @returns Every `combat:start`/`end`/`round-start`/`round-end`/`turn-start`/`turn-end`/`rewind`
 * event this touch's transition produced, in derivation order.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import type { WireCommand, CombatEngine } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * declare const touch: { id: string; b: CombatEngine | undefined; a: CombatEngine | undefined };
 * const events = processCombat(touch, cmd, (id) => store.get(id), store);
 * events.map((e) => e.name); // ["combat:round-start", "combat:turn-start"]
 * ```
 */
function processCombat(
  touch: CombatTouch,
  cmd: WireCommand,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): CombatHookEvent[] {
  const { id, b, a } = touch;
  const events: CombatHookEvent[] = [];

  if (a && b) {
    const roundRewind = a.round < b.round;
    let turnRewind = false;
    if (!roundRewind && a.round === b.round && b.turn != null && a.turn != null) {
      const bi = a.order.indexOf(b.turn);
      const ai = a.order.indexOf(a.turn);
      if (bi !== -1 && ai !== -1 && ai < bi) turnRewind = true;
    }
    if (roundRewind || turnRewind) {
      events.push({ name: "combat:rewind", payload: { combatId: id, round: a.round, turn: a.turn } });
      return events;
    }
  }

  if ((!b || !b.active) && a?.active) {
    events.push({
      name: "combat:start",
      payload: { combatId: id, sceneId: a.scene_id, round: a.round, resumed: b?.turn != null },
    });
  }

  let prev = { round: b?.round ?? 0, turn: (b?.turn ?? null) as string | null };
  for (const step of turnWalk(touch, collectHistoryTouch(cmd, id, before, after))) {
    if (prev.turn != null) events.push(turnEvent(id, "combat:turn-end", prev.round, prev.turn, before, after));
    pushRounds(events, id, prev.round, step.round);
    events.push(turnEvent(id, "combat:turn-start", step.round, step.turn, before, after));
    prev = step;
  }
  // The clock stopped holding a turn (the combat was deleted, or `turn` was cleared): the turn
  // it last held ended without a successor.
  if (a?.turn == null && prev.turn != null) events.push(turnEvent(id, "combat:turn-end", prev.round, prev.turn, before, after));
  if (a) pushRounds(events, id, prev.round, a.round);

  if (b?.active && a && !a.active) {
    events.push({ name: "combat:end", payload: { combatId: id, sceneId: b.scene_id, round: b.round, reason: "paused" } });
  } else if (b?.active && !a) {
    events.push({ name: "combat:end", payload: { combatId: id, sceneId: b.scene_id, round: b.round, reason: "ended" } });
  }

  return events;
}

/** Derives `combat:effect-tick`/`combat:effect-expired` hook events for one combat's command.
 * @param cmd The applied command whose ops to scan for effect changes.
 * @param attributed The combat this command's effect changes are attributed to.
 * @param before Looks up a document's pre-image by id.
 * @param after The post-command document view (a `DocumentStore`-shaped reader).
 * @returns Every `combat:effect-tick`/`combat:effect-expired` event the command's effect
 * mutations produced, in op order.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import type { WireCommand, CombatEngine } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * declare const attributed: { id: string; b: CombatEngine | undefined; a: CombatEngine | undefined };
 * const events = deriveEffectEvents(cmd, attributed, (id) => store.get(id), store);
 * events[0]?.name; // "combat:effect-tick"
 * ```
 */
function deriveEffectEvents(
  cmd: WireCommand,
  attributed: CombatTouch,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): CombatHookEvent[] {
  const round = attributed.a?.round ?? attributed.b?.round ?? 0;
  const combatId = attributed.id;
  const events: CombatHookEvent[] = [];
  for (const op of cmd.ops as WireOperation[]) {
    if (op.op !== "update") continue;
    for (const change of op.changes) {
      const m = EFFECT_PATH.exec(change.path);
      if (!m) continue;
      const prefix = m[1];
      const leaf = m[2];
      const hostDoc = after.get(op.doc_id) ?? before(op.doc_id);
      const effectId = hostDoc ? ((getPointer(hostDoc, prefix + "/id") as string | undefined) ?? null) : null;
      if (leaf === "duration/remaining") {
        const newValue = change.new;
        const oldValue = change.old;
        if (typeof newValue === "number" && (oldValue == null || newValue < (oldValue as number))) {
          events.push({
            name: "combat:effect-tick",
            payload: { combatId, round, hostId: op.doc_id, path: prefix, effectId, remaining: newValue },
          });
        }
      } else if (leaf === "active") {
        if (change.old === true && change.new === false) {
          events.push({
            name: "combat:effect-expired",
            payload: { combatId, round, hostId: op.doc_id, path: prefix, effectId },
          });
        }
      }
    }
  }
  return events;
}

/** Pure derivation of every `combat:*` event one applied command produced, in the documented
 * order: per touched combat, rewind XOR (start; then, per turn boundary the command crossed,
 * turn-end of the turn being left, the round-end/round-start pairs up to the boundary's round,
 * turn-start; a trailing turn-end when the clock stopped holding a turn; end); then effect
 * events attributed to the one combat the command touched (the active one if several, else
 * the first encountered). The boundaries crossed are the `combat-history` records the command
 * appended when the recipient received that write, else the combat document's own endpoints
 * (`turnWalk`). A command that touches no `combat` document (create/update/delete) produces no
 * events at all, including no effect events, even if it also happens to touch an embedded
 * effect field.
 * @param before Resolves a document id to its pre-command state (the `WorldSession` pre-image
 * map built only for commands `commandTouchesCombat` admits).
 * @param cmd The just-applied, sequenced command.
 * @param after The document view AFTER `cmd` has applied (e.g. the `DocumentStore` itself).
 * @returns The ordered list of hook events to emit for this command.
 * @example
 * ```ts
 * import { DocumentStore } from "@shadowcat/core";
 * import { deriveCombatHookEvents } from "@shadowcat/core";
 * import type { WireCommand } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * declare const cmd: WireCommand;
 * deriveCombatHookEvents((id) => store.get(id), cmd, store);
 * ```
 */
export function deriveCombatHookEvents(
  before: (id: string) => WireDocument | undefined,
  cmd: WireCommand,
  after: ReadableDocuments,
): CombatHookEvent[] {
  const touches = collectCombatTouches(cmd, before, after);
  if (touches.size === 0) return [];

  const events: CombatHookEvent[] = [];
  for (const touch of touches.values()) {
    events.push(...processCombat(touch, cmd, before, after));
  }

  let attributed: CombatTouch | undefined;
  for (const t of touches.values()) {
    if (t.a?.active) {
      attributed = t;
      break;
    }
  }
  if (!attributed) attributed = touches.values().next().value;
  if (attributed) events.push(...deriveEffectEvents(cmd, attributed, before, after));

  return events;
}

/** Chains `emitInfo` calls for a derived event list onto an internal promise queue, so a
 * listener that awaits never observes a later command's event before an earlier one's, while
 * remaining fire-and-forget from the caller (`WorldSession.onCommand` never awaits `emit`). A
 * throwing listener is already isolated by `HookBus.emitInfo`; it does not stall this queue. */
export class CombatHookEmitter {
  /** The emission queue: each `emit` call chains its `emitInfo` calls onto this promise. */
  #tail: Promise<void> = Promise.resolve();

  /**
   * Constructs an emitter bound to one `HookBus`.
   * @param hooks The bus to emit on (must already have `defineCombatHooks` called against it).
   * @param logger Diagnostic sink for a queue-level failure (never expected in practice, since
   * `HookBus.emitInfo` itself never rejects).
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   * import { CombatHookEmitter, defineCombatHooks } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * defineCombatHooks(hooks);
   * const emitter = new CombatHookEmitter(hooks, silentLogger);
   * ```
   */
  constructor(
    private readonly hooks: HookBus,
    private readonly logger: Logger,
  ) {}

  /** Enqueues every event in `events`, in order, onto the emission queue.
   * @param events The events to emit, in the order `deriveCombatHookEvents` returned them.
   * @example
   * ```ts
   * import { HookBus, silentLogger } from "@shadowcat/core";
   * import { CombatHookEmitter, defineCombatHooks } from "@shadowcat/core";
   *
   * const hooks = new HookBus(silentLogger);
   * defineCombatHooks(hooks);
   * const emitter = new CombatHookEmitter(hooks, silentLogger);
   * emitter.emit([]);
   * ```
   */
  emit(events: CombatHookEvent[]): void {
    for (const ev of events) {
      this.#tail = this.#tail
        .then(() => this.hooks.emitInfo(ev.name, ev.payload))
        .catch((err: unknown) => this.logger.error("combat hook emission queue failed", err));
    }
  }
}
