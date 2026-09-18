import {
  MESSAGE_DOC_TYPE,
  parseMessageEngine,
  isKnownSegment,
  type ReadableDocuments,
  type RollOutcome,
  type WireDocument,
  type TableDrawSegment,
} from "@shadowcat/core";

/** Cap on rolls tumbling at once; a queue of rolls shares the tray, so at most this many
 * tumble concurrently. */
export const MAX_CONCURRENT_ROLLS = 3;

/** Cap on dice rendered per roll; the remainder is a "+N" badge. */
export const MAX_DICE_PER_ROLL = 30;

/** One roll's play instruction: identity, replay key, and its outcome. */
export interface RollPlay {
  /** The roll's stable id (`roll_embed.roll_id` / `table_draw.roll_id`). */
  rollId: string;
  /** `recalc_history.length` at scan time (0 for a `table_draw`, which carries no recalc
   * history — a table draw is not recalculable). A later scan with a HIGHER count re-plays
   * (the recalc case); the same or lower count never re-plays. */
  recalcCount: number;
  /** The roll's full deterministic outcome. */
  outcome: RollOutcome;
}

/** Per-mount trigger bookkeeping: every roll id already accounted for, and the
 * `recalcCount` it was last accounted for at. */
export interface TriggerState {
  /** rollId -> last-seen recalcCount. */
  seen: Map<string, number>;
}

/** Builds an empty {@link TriggerState}.
 * @returns A fresh, empty trigger state.
 * @example
 * ```ts
 * import { createTriggerState } from "@shadowcat/module-dice-3d";
 *
 * createTriggerState();
 * ```
 */
export function createTriggerState(): TriggerState {
  return { seen: new Map() };
}

/** Recursively collects every roll (a `roll_embed` or `table_draw`, including nested draws)
 * reachable from one message document's content.
 * @param doc The candidate document; a non-`message` `doc_type` or a malformed engine body
 * yields no rolls (the `parseMessageEngine` fail-closed read).
 * @returns Every roll the document carries, in content order, nested draws after their parent.
 * @example
 * ```ts
 * // private helper; not part of the public API — called by `seedFromSnapshot`/`scanForPlays`
 * declare const doc: import("@shadowcat/core").WireDocument;
 * rollsIn(doc);
 * ```
 */
function rollsIn(doc: WireDocument): RollPlay[] {
  const engine = parseMessageEngine(doc);
  if (!engine) return [];
  const out: RollPlay[] = [];
  const visitDraw = (draw: TableDrawSegment): void => {
    out.push({ rollId: draw.roll_id, recalcCount: 0, outcome: draw.outcome });
    for (const nested of draw.row?.nested ?? []) visitDraw(nested);
  };
  for (const seg of engine.content) {
    if (!isKnownSegment(seg)) continue;
    if (seg.kind === "roll_embed" && seg.roll_id) {
      out.push({ rollId: seg.roll_id, recalcCount: seg.recalc_history?.length ?? 0, outcome: seg.outcome });
    } else if (seg.kind === "table_draw") {
      visitDraw(seg);
    }
  }
  return out;
}

/**
 * Seeds `state` from every `message` document already in the store — the cold-start
 * snapshot IS history: it is seeded into `DocumentStore` before the socket connects,
 * so nothing already present at mount ever plays. Call once, before the first
 * {@link scanForPlays}.
 * @param state The trigger state to seed (mutated in place).
 * @param documents The document view to seed from.
 * @example
 * ```ts
 * import { createTriggerState, seedFromSnapshot } from "@shadowcat/module-dice-3d";
 * declare const documents: import("@shadowcat/core").ReadableDocuments;
 *
 * const state = createTriggerState();
 * seedFromSnapshot(state, documents);
 * ```
 */
export function seedFromSnapshot(state: TriggerState, documents: ReadableDocuments): void {
  for (const doc of documents.query(MESSAGE_DOC_TYPE)) {
    for (const play of rollsIn(doc)) {
      state.seen.set(play.rollId, play.recalcCount);
    }
  }
}

/**
 * Scans every `message` document currently in the store for rolls not yet accounted for in
 * `state` (new since the last scan, or recalculated to a higher `recalcCount`), returning
 * exactly the plays to trigger — and marks them all seen (at their new `recalcCount`) so a
 * repeat scan never re-plays the same state twice. Marks a play's identity as seen even when
 * `dice3dEnabled` is false at the call site — the CALLER decides whether to actually render;
 * this function's bookkeeping never diverges from what happened on the wire, so re-enabling
 * later never replays history.
 * @param state The trigger state (mutated in place).
 * @param documents The document view to scan.
 * @returns Every new/recalculated play, in no particular order.
 * @example
 * ```ts
 * import { createTriggerState, scanForPlays } from "@shadowcat/module-dice-3d";
 * declare const documents: import("@shadowcat/core").ReadableDocuments;
 *
 * const state = createTriggerState();
 * scanForPlays(state, documents);
 * ```
 */
export function scanForPlays(state: TriggerState, documents: ReadableDocuments): RollPlay[] {
  const out: RollPlay[] = [];
  for (const doc of documents.query(MESSAGE_DOC_TYPE)) {
    for (const play of rollsIn(doc)) {
      const last = state.seen.get(play.rollId);
      if (last === undefined || play.recalcCount > last) {
        out.push(play);
        state.seen.set(play.rollId, play.recalcCount);
      }
    }
  }
  return out;
}

/** One roll queued or actively tumbling, with its dice capped at {@link MAX_DICE_PER_ROLL}. */
export interface QueuedRoll {
  /** The roll's stable id. */
  rollId: string;
  /** The roll's full outcome (used for its per-die records). */
  outcome: RollOutcome;
  /** Count of dice beyond {@link MAX_DICE_PER_ROLL}, rendered as a "+N" badge; 0 if none. */
  overflow: number;
}

/** Builds a {@link QueuedRoll} from a {@link RollPlay}, capping its dice count.
 * @param play The play to wrap.
 * @returns The queued roll, with `overflow` set when `play.outcome.records.length` exceeds
 * {@link MAX_DICE_PER_ROLL}.
 * @example
 * ```ts
 * import { toQueuedRoll } from "@shadowcat/module-dice-3d";
 * declare const play: import("@shadowcat/module-dice-3d").RollPlay;
 *
 * toQueuedRoll(play);
 * ```
 */
export function toQueuedRoll(play: RollPlay): QueuedRoll {
  const overflow = Math.max(0, play.outcome.records.length - MAX_DICE_PER_ROLL);
  return { rollId: play.rollId, outcome: play.outcome, overflow };
}

/** Queue state: dice actively tumbling (`<= MAX_CONCURRENT_ROLLS`) and rolls waiting their
 * turn. */
export interface RollQueue {
  /** Currently-tumbling rolls. */
  active: QueuedRoll[];
  /** Rolls waiting for a tray slot, in arrival order. */
  pending: QueuedRoll[];
}

/** Builds an empty {@link RollQueue}.
 * @returns A fresh, empty queue.
 * @example
 * ```ts
 * import { createRollQueue } from "@shadowcat/module-dice-3d";
 *
 * createRollQueue();
 * ```
 */
export function createRollQueue(): RollQueue {
  return { active: [], pending: [] };
}

/** Enqueues `next`: onto `active` if a tray slot is free, else onto `pending`.
 * @param queue The current queue.
 * @param next The roll to enqueue.
 * @returns The new queue state (does not mutate `queue`).
 * @example
 * ```ts
 * import { createRollQueue, enqueueRoll, toQueuedRoll } from "@shadowcat/module-dice-3d";
 * declare const play: import("@shadowcat/module-dice-3d").RollPlay;
 *
 * enqueueRoll(createRollQueue(), toQueuedRoll(play));
 * ```
 */
export function enqueueRoll(queue: RollQueue, next: QueuedRoll): RollQueue {
  if (queue.active.length < MAX_CONCURRENT_ROLLS) {
    return { active: [...queue.active, next], pending: queue.pending };
  }
  return { active: queue.active, pending: [...queue.pending, next] };
}

/** The result of a {@link dequeueRoll}: the new queue state plus the pending roll promoted
 * into the freed tray slot, if one was waiting (a named interface rather than an inline
 * object-literal type so both properties can be documented). */
export interface DequeueResult {
  /** The new queue state. */
  queue: RollQueue;
  /** The oldest pending roll promoted into the freed slot, or `null` when none was waiting. */
  promoted: QueuedRoll | null;
}

/** Removes `rollId` from `active` (its fade/dismiss completed) and promotes the oldest
 * pending roll into the freed slot, if any.
 * @param queue The current queue.
 * @param rollId The roll id to remove from `active`.
 * @returns The new queue state and the promoted roll, if one was waiting.
 * @example
 * ```ts
 * import { createRollQueue, dequeueRoll } from "@shadowcat/module-dice-3d";
 *
 * dequeueRoll(createRollQueue(), "r1");
 * ```
 */
export function dequeueRoll(
  queue: RollQueue,
  rollId: string,
): DequeueResult {
  const active = queue.active.filter((r) => r.rollId !== rollId);
  if (active.length < MAX_CONCURRENT_ROLLS && queue.pending.length > 0) {
    const [promoted, ...rest] = queue.pending;
    return { queue: { active: [...active, promoted], pending: rest }, promoted };
  }
  return { queue: { active, pending: queue.pending }, promoted: null };
}
