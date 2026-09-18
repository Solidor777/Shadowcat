// @vitest-environment node
import { describe, it, expect } from "vitest";
import { DocumentStore, buildChannelRegistryDoc, type RollOutcome, type WireDocument } from "@shadowcat/core";
import {
  createTriggerState,
  seedFromSnapshot,
  scanForPlays,
  createRollQueue,
  enqueueRoll,
  dequeueRoll,
  toQueuedRoll,
  MAX_CONCURRENT_ROLLS,
  MAX_DICE_PER_ROLL,
  type RollPlay,
} from "./trigger";

function outcome(recordCount = 1): RollOutcome {
  return {
    total: 1,
    records: Array.from({ length: recordCount }, () => ({
      value: 1, natural: 1, kept: true, exploded: false, crit_success: false,
      crit_fail: false, expertise: 0, group_index: 0, symbols: [],
    })),
    crit_successes: 0, crit_fails: 0, positive_counter: 0, negative_counter: 0,
    symbol_counts: {}, labeled_consts: [],
  };
}

function messageDoc(id: string, content: unknown[]): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "message", schema_version: 1,
    name: null, source: null, owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {}, parent_id: null,
    engine: { channel: "general", user_owner: "u1", kind: "roll", audience: { kind: "public" }, content },
    system: {}, created_at: 1, updated_at: 1,
  };
}

function rollEmbedDoc(id: string, rollId: string, recalcCount: number): WireDocument {
  return messageDoc(id, [{
    kind: "roll_embed", formula: "1d20", outcome: outcome(), roll_id: rollId,
    recalc_history: recalcCount > 0 ? Array.from({ length: recalcCount }, () => ({ previous_outcome: outcome(), recalculated_by: "gm", recalculated_at: 0 })) : undefined,
  }]);
}

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("seedFromSnapshot + scanForPlays", () => {
  it("a message present at mount never plays", () => {
    const store = storeWith(rollEmbedDoc("m1", "r1", 0));
    const state = createTriggerState();
    seedFromSnapshot(state, store);
    expect(scanForPlays(state, store)).toEqual([]);
  });

  it("a message arriving after mount plays exactly once", () => {
    const store = storeWith();
    const state = createTriggerState();
    seedFromSnapshot(state, store);
    store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: rollEmbedDoc("m1", "r1", 0) }] });
    const plays = scanForPlays(state, store);
    expect(plays.map((p) => p.rollId)).toEqual(["r1"]);
    expect(scanForPlays(state, store)).toEqual([]); // a repeat scan never re-plays
  });

  it("a table_draw plays, including its nested draws", () => {
    const draw = messageDoc("m1", [{
      kind: "table_draw", table_id: "t1", table_name: "Loot", roll_id: "d1", formula: "1d6",
      outcome: outcome(), row: { index: 0, label: "chest", content: [], nested: [
        { kind: "table_draw", table_id: "t2", table_name: "Gems", roll_id: "d2", formula: "1d4", outcome: outcome(), row: null },
      ] },
    }]);
    const store = storeWith();
    const state = createTriggerState();
    seedFromSnapshot(state, store);
    store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: draw }] });
    const plays = scanForPlays(state, store).map((p) => p.rollId).sort();
    expect(plays).toEqual(["d1", "d2"]);
  });

  it("a recalc re-plays under its new recalcCount", () => {
    const store = storeWith(rollEmbedDoc("m1", "r1", 0));
    const state = createTriggerState();
    seedFromSnapshot(state, store);
    store.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: rollEmbedDoc("m1", "r1", 1) }] });
    const plays = scanForPlays(state, store);
    expect(plays).toHaveLength(1);
    expect(plays[0].recalcCount).toBe(1);
  });

  it("a non-message, non-roll document contributes nothing", () => {
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const state = createTriggerState();
    seedFromSnapshot(state, store);
    expect(scanForPlays(state, store)).toEqual([]);
  });
});

describe("RollQueue", () => {
  const play = (rollId: string, records = 1): RollPlay => ({ rollId, recalcCount: 0, outcome: outcome(records) });

  it("caps concurrent tumbles at MAX_CONCURRENT_ROLLS, queuing the rest", () => {
    let queue = createRollQueue();
    for (let i = 0; i < MAX_CONCURRENT_ROLLS + 2; i++) {
      queue = enqueueRoll(queue, toQueuedRoll(play(`r${i}`)));
    }
    expect(queue.active).toHaveLength(MAX_CONCURRENT_ROLLS);
    expect(queue.pending).toHaveLength(2);
  });

  it("dequeue promotes the oldest pending roll into the freed slot", () => {
    let queue = createRollQueue();
    for (let i = 0; i < MAX_CONCURRENT_ROLLS + 1; i++) {
      queue = enqueueRoll(queue, toQueuedRoll(play(`r${i}`)));
    }
    const { queue: after, promoted } = dequeueRoll(queue, "r0");
    expect(promoted?.rollId).toBe(`r${MAX_CONCURRENT_ROLLS}`);
    expect(after.active.map((r) => r.rollId)).not.toContain("r0");
    expect(after.pending).toHaveLength(0);
  });

  it("a roll over MAX_DICE_PER_ROLL renders capped dice plus an overflow badge count", () => {
    const queued = toQueuedRoll(play("big", MAX_DICE_PER_ROLL + 7));
    expect(queued.overflow).toBe(7);
  });

  it("a roll at or under the cap has no overflow", () => {
    expect(toQueuedRoll(play("small", MAX_DICE_PER_ROLL)).overflow).toBe(0);
  });
});
