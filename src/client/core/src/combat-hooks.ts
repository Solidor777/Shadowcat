// The combat clock's hook surface: pure derivation of typed events from an applied command's
// delta over the combat/combatant documents it touched, plus the emitter that chains them onto
// a HookBus in seq order. No formula evaluation and no server round-trip here -- every event is
// read straight off the same optimistic/authoritative documents WorldSession already applies.
import type { HookBus, CoreHooks } from "./hooks";
import type { Logger } from "./logger";
import type { ReadableDocuments } from "./store";
import { getPointer } from "./store";
import type { WireDocument, WireCommand, WireOperation } from "./wire";
import type { CombatEngine, CombatantEngine } from "./scene-docs";

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

function combatantKindOf(doc: WireDocument | undefined): "actor" | "event" | null {
  const engine = doc?.engine as CombatantEngine | undefined;
  return engine?.kind.type ?? null;
}

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

/** One intermediate event combatant's turn, derived from a `combatant` op inside the same
 * command as its parent combat's transition. */
interface IntermediateEvent {
  /** The intermediate event combatant's document id. */
  id: string;
  /** The combatant's pre-image (its state before the command applied). */
  doc: WireDocument;
}

function gatherIntermediateEvents(
  cmd: WireCommand,
  combatId: string,
  before: (id: string) => WireDocument | undefined,
  after: ReadableDocuments,
): IntermediateEvent[] {
  const out: IntermediateEvent[] = [];
  for (const op of cmd.ops) {
    if (op.op === "update") {
      const b = before(op.doc_id);
      if (!b || b.doc_type !== "combatant" || b.parent_id !== combatId) continue;
      const bEngine = b.engine as CombatantEngine;
      if (bEngine.kind.type !== "event") continue;
      const beforeLifespan = bEngine.kind.lifespan;
      const aDoc = after.get(op.doc_id);
      const aEngine = aDoc?.engine as CombatantEngine | undefined;
      const afterLifespan = aEngine?.kind.type === "event" ? aEngine.kind.lifespan : undefined;
      if (afterLifespan != null && beforeLifespan != null && afterLifespan < beforeLifespan) {
        out.push({ id: op.doc_id, doc: b });
      }
    } else if (op.op === "delete") {
      const doc = op.doc;
      if (doc.doc_type !== "combatant" || doc.parent_id !== combatId) continue;
      const engine = doc.engine as CombatantEngine;
      if (engine.kind.type !== "event") continue;
      out.push({ id: doc.id, doc });
    }
  }
  return out;
}

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

  if (b?.turn != null && a?.turn !== b.turn) {
    const doc = after.get(b.turn) ?? before(b.turn);
    events.push({
      name: "combat:turn-end",
      payload: { combatId: id, round: b.round, combatantId: b.turn, kind: combatantKindOf(doc) },
    });
  }

  if (a) {
    for (let r = b?.round ?? 0; r < a.round; r++) {
      if (r > 0) events.push({ name: "combat:round-end", payload: { combatId: id, round: r } });
      events.push({ name: "combat:round-start", payload: { combatId: id, round: r + 1 } });
    }
  }

  if (a && b) {
    const intermediates = gatherIntermediateEvents(cmd, id, before, after)
      .filter((e) => e.id !== a.turn && e.id !== b.turn)
      .sort((x, y) => b.order.indexOf(x.id) - b.order.indexOf(y.id));
    for (const ev of intermediates) {
      events.push({
        name: "combat:turn-start",
        payload: { combatId: id, round: a.round, combatantId: ev.id, kind: "event" },
      });
      events.push({
        name: "combat:turn-end",
        payload: { combatId: id, round: a.round, combatantId: ev.id, kind: "event" },
      });
    }
  }

  if (a?.turn != null && a.turn !== b?.turn) {
    const doc = after.get(a.turn);
    events.push({
      name: "combat:turn-start",
      payload: { combatId: id, round: a.round, combatantId: a.turn, kind: combatantKindOf(doc) },
    });
  }

  if (b?.active && a && !a.active) {
    events.push({ name: "combat:end", payload: { combatId: id, sceneId: b.scene_id, round: b.round, reason: "paused" } });
  } else if (b?.active && !a) {
    events.push({ name: "combat:end", payload: { combatId: id, sceneId: b.scene_id, round: b.round, reason: "ended" } });
  }

  return events;
}

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
 * order: per touched combat, rewind XOR (start, turn-end, round-end/round-start pairs,
 * intermediate event turns, turn-start, end); then effect events attributed to the one combat
 * the command touched (the active one if several, else the first encountered). A command that
 * touches no `combat` document (create/update/delete) produces no events at all, including no
 * effect events, even if it also happens to touch an embedded effect field.
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
