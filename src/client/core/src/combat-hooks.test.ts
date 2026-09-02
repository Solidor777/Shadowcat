import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import { HookBus } from "./hooks";
import { silentLogger } from "./logger";
import type { WireDocument, WireCommand, WireOperation, WireFieldChange } from "./wire";
import { buildCombatDoc, buildCombatantDoc, envelope } from "./scene-docs";
import type { CombatEngine, CombatantEngine } from "./scene-docs";
import {
  deriveCombatHookEvents,
  defineCombatHooks,
  commandTouchesCombat,
  CombatHookEmitter,
  type CombatHookEvent,
} from "./combat-hooks";

const WORLD = "world-1";

function combat(overrides: Partial<CombatEngine>, id = "combat-1"): WireDocument {
  const engine: CombatEngine = {
    scene_id: "scene-1",
    active: false,
    round: 0,
    turn: null,
    turn_control: "owner_may_end",
    order: [],
    movement: { resource: null, interpretation: "per_cell", enforcement: "none" },
    effect_cleanup: true,
    rewind_restore: true,
    forward_restore: false,
    effect_lifecycle: { onCombatEnd: null, onTurnEnd: null, onAdvance: null },
    ...overrides,
  };
  return buildCombatDoc(WORLD, engine, id);
}

function actorCombatant(id: string, combatId = "combat-1"): WireDocument {
  const engine: CombatantEngine = {
    kind: { type: "actor", token_id: null, actor_id: null },
    initiative: 10,
    tiebreak: 0,
    resources: {},
  };
  return buildCombatantDoc(WORLD, combatId, engine, { id });
}

function hostWithEffect(hostId: string, effectId: string, remaining: number, active: boolean): WireDocument {
  const effect = envelope(WORLD, "effect", hostId, {}, effectId, { duration: { remaining }, active });
  const host = actorCombatant("host-anchor");
  return { ...host, id: hostId, embedded: { effects: [effect] } };
}

function eventCombatant(id: string, lifespan: number | null, combatId = "combat-1"): WireDocument {
  const engine: CombatantEngine = {
    kind: { type: "event", lifespan, message: null },
    initiative: 0,
    tiebreak: 0,
    resources: {},
  };
  return buildCombatantDoc(WORLD, combatId, engine, { id, name: "event" });
}

/** Builds a store seeded with `docs`, then applies one synthetic command built from `ops`
 * against a mutated clone (never against the seeded originals), returning the pre-image lookup
 * (`before`), the command itself, and the seeded store mutated to the post-state (`after`). */
function transition(
  docs: WireDocument[],
  ops: WireOperation[],
): { before: (id: string) => WireDocument | undefined; cmd: WireCommand; after: DocumentStore } {
  const store = new DocumentStore();
  store.seedDocuments(docs);
  const preimages = new Map<string, WireDocument | undefined>();
  for (const op of ops) {
    const id = op.op === "update" ? op.doc_id : op.doc.id;
    preimages.set(id, store.get(id));
  }
  const cmd: WireCommand = { seq: 1, world_id: WORLD, author: "gm", ts: 0, ops };
  store.applyCommand(cmd);
  return { before: (id) => preimages.get(id), cmd, after: store };
}

function updateEngine(doc: WireDocument, patch: Partial<Record<string, unknown>>): WireOperation {
  const changes: WireFieldChange[] = Object.entries(patch).map(([key, value]) => ({
    path: `/engine/${key}`,
    old: (doc.engine as Record<string, unknown>)[key],
    new: value,
  }));
  return { op: "update", doc_id: doc.id, changes };
}

describe("deriveCombatHookEvents", () => {
  it("initial start: round 0->1, turn null->A", () => {
    const c = combat({ active: false, round: 0, turn: null, order: ["A", "B"] });
    const a = actorCombatant("A");
    const b = actorCombatant("B");
    const { before, cmd, after } = transition([c, a, b], [
      updateEngine(c, { active: true, round: 1, turn: "A" }),
    ]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:start", payload: { combatId: "combat-1", sceneId: "scene-1", round: 1, resumed: false } },
      { name: "combat:round-start", payload: { combatId: "combat-1", round: 1 } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
    ]);
  });

  it("resume: active false->true, turn already set", () => {
    const c = combat({ active: false, round: 1, turn: "A", order: ["A", "B"] });
    const a = actorCombatant("A");
    const { before, cmd, after } = transition([c, a], [updateEngine(c, { active: true })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:start", payload: { combatId: "combat-1", sceneId: "scene-1", round: 1, resumed: true } },
    ]);
  });

  it("advance A->B same round", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A", "B"] });
    const a = actorCombatant("A");
    const b = actorCombatant("B");
    const { before, cmd, after } = transition([c, a, b], [updateEngine(c, { turn: "B" })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "B", kind: "actor" } },
    ]);
  });

  it("wrap B->A, new round", () => {
    const c = combat({ active: true, round: 1, turn: "B", order: ["A", "B"] });
    const a = actorCombatant("A");
    const b = actorCombatant("B");
    const { before, cmd, after } = transition([c, a, b], [updateEngine(c, { turn: "A", round: 2 })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "B", kind: "actor" } },
      { name: "combat:round-end", payload: { combatId: "combat-1", round: 1 } },
      { name: "combat:round-start", payload: { combatId: "combat-1", round: 2 } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 2, combatantId: "A", kind: "actor" } },
    ]);
  });

  it("event intermediate: A->C with event E's lifespan decreasing 2->1", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A", "E", "B", "C"] });
    const a = actorCombatant("A");
    const e = eventCombatant("E", 2);
    const b = actorCombatant("B");
    const cc = actorCombatant("C");
    const { before, cmd, after } = transition([c, a, e, b, cc], [
      updateEngine(c, { turn: "C" }),
      {
        op: "update",
        doc_id: "E",
        changes: [{ path: "/engine/kind/lifespan", old: 2, new: 1 }],
      },
    ]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "E", kind: "event" } },
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "E", kind: "event" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "C", kind: "actor" } },
    ]);
  });

  it("event removal: E deleted mid-turn, kind resolved from the delete pre-image", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A", "E", "C"] });
    const a = actorCombatant("A");
    const e = eventCombatant("E", 1);
    const cc = actorCombatant("C");
    const { before, cmd, after } = transition([c, a, e, cc], [
      updateEngine(c, { turn: "C" }),
      { op: "delete", doc: e },
    ]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "E", kind: "event" } },
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "E", kind: "event" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "C", kind: "actor" } },
    ]);
  });

  it("hidden turn: turn names an id absent from the store", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A", "ghost"] });
    const a = actorCombatant("A");
    const { before, cmd, after } = transition([c, a], [updateEngine(c, { turn: "ghost" })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
      { name: "combat:turn-start", payload: { combatId: "combat-1", round: 1, combatantId: "ghost", kind: null } },
    ]);
  });

  it("pause: active true->false, turn unchanged", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A"] });
    const a = actorCombatant("A");
    const { before, cmd, after } = transition([c, a], [updateEngine(c, { active: false })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:end", payload: { combatId: "combat-1", sceneId: "scene-1", round: 1, reason: "paused" } },
    ]);
  });

  it("end: deleting an active combat", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A"] });
    const a = actorCombatant("A");
    const { before, cmd, after } = transition([c, a], [{ op: "delete", doc: c }]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:turn-end", payload: { combatId: "combat-1", round: 1, combatantId: "A", kind: "actor" } },
      { name: "combat:end", payload: { combatId: "combat-1", sceneId: "scene-1", round: 1, reason: "ended" } },
    ]);
  });

  it("rewind across a round", () => {
    const c = combat({ active: true, round: 2, turn: "B", order: ["A", "B"] });
    const a = actorCombatant("A");
    const b = actorCombatant("B");
    const { before, cmd, after } = transition([c, a, b], [updateEngine(c, { round: 1, turn: "A" })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:rewind", payload: { combatId: "combat-1", round: 1, turn: "A" } },
    ]);
  });

  it("rewind within a round: turn moves backward in order without a round change", () => {
    const c = combat({ active: true, round: 1, turn: "B", order: ["A", "B", "C"] });
    const a = actorCombatant("A");
    const b = actorCombatant("B");
    const cc = actorCombatant("C");
    const { before, cmd, after } = transition([c, a, b, cc], [updateEngine(c, { turn: "A" })]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:rewind", payload: { combatId: "combat-1", round: 1, turn: "A" } },
    ]);
  });

  it("effect tick: duration/remaining decreases, attributed to the touched combat", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A"] });
    const host: WireDocument = hostWithEffect("host-1", "eff-1", 3, true);
    const { before, cmd, after } = transition([c, host], [
      updateEngine(c, { turn: "A" }), // no-op combat touch to anchor attribution
      {
        op: "update",
        doc_id: "host-1",
        changes: [{ path: "/embedded/effects/0/engine/duration/remaining", old: 3, new: 2 }],
      },
    ]);
    const events = deriveCombatHookEvents(before, cmd, after);
    expect(events).toContainEqual({
      name: "combat:effect-tick",
      payload: { combatId: "combat-1", round: 1, hostId: "host-1", path: "/embedded/effects/0", effectId: "eff-1", remaining: 2 },
    });
  });

  it("effect expiry: active true->false", () => {
    const c = combat({ active: true, round: 1, turn: "A", order: ["A"] });
    const host: WireDocument = hostWithEffect("host-1", "eff-1", 0, true);
    const { before, cmd, after } = transition([c, host], [
      updateEngine(c, { turn: "A" }),
      {
        op: "update",
        doc_id: "host-1",
        changes: [{ path: "/embedded/effects/0/engine/active", old: true, new: false }],
      },
    ]);
    const events = deriveCombatHookEvents(before, cmd, after);
    expect(events).toContainEqual({
      name: "combat:effect-expired",
      payload: { combatId: "combat-1", round: 1, hostId: "host-1", path: "/embedded/effects/0", effectId: "eff-1" },
    });
  });

  it("effect edit with no combat op in the command yields no events at all", () => {
    const host: WireDocument = hostWithEffect("host-1", "eff-1", 3, true);
    const { before, cmd, after } = transition([host], [
      {
        op: "update",
        doc_id: "host-1",
        changes: [{ path: "/embedded/effects/0/engine/duration/remaining", old: 3, new: 2 }],
      },
    ]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual([]);
  });

  it("a CombatStart swap: end{paused} for the old, start for the new, in op order", () => {
    const oldCombat = combat({ active: true, round: 3, turn: "A", order: ["A"] }, "combat-old");
    const newCombat = combat({ active: false, round: 0, turn: null, order: ["B"] }, "combat-new");
    const a = actorCombatant("A", "combat-old");
    const b = actorCombatant("B", "combat-new");
    const { before, cmd, after } = transition([oldCombat, newCombat, a, b], [
      updateEngine(oldCombat, { active: false }),
      updateEngine(newCombat, { active: true, round: 1, turn: "B" }),
    ]);
    expect(deriveCombatHookEvents(before, cmd, after)).toEqual<CombatHookEvent[]>([
      { name: "combat:end", payload: { combatId: "combat-old", sceneId: "scene-1", round: 3, reason: "paused" } },
      { name: "combat:start", payload: { combatId: "combat-new", sceneId: "scene-1", round: 1, resumed: false } },
      { name: "combat:round-start", payload: { combatId: "combat-new", round: 1 } },
      { name: "combat:turn-start", payload: { combatId: "combat-new", round: 1, combatantId: "B", kind: "actor" } },
    ]);
  });
});

describe("commandTouchesCombat", () => {
  it("true for a combat/combatant op, an embedded effect change; false otherwise", () => {
    const store = new DocumentStore();
    const c = combat({});
    store.seedDocuments([c]);
    expect(
      commandTouchesCombat({ seq: 1, world_id: WORLD, author: "gm", ts: 0, ops: [{ op: "create", doc: c }] }, store),
    ).toBe(true);
    expect(
      commandTouchesCombat(
        { seq: 1, world_id: WORLD, author: "gm", ts: 0, ops: [updateEngine(c, { round: 1 })] },
        store,
      ),
    ).toBe(true);
    const token: WireDocument = { ...c, id: "tok-1", doc_type: "token" } as unknown as WireDocument;
    store.seedDocuments([c, token]);
    expect(
      commandTouchesCombat(
        {
          seq: 1,
          world_id: WORLD,
          author: "gm",
          ts: 0,
          ops: [{ op: "update", doc_id: "tok-1", changes: [{ path: "/engine/x", old: 0, new: 1 }] }],
        },
        store,
      ),
    ).toBe(false);
    expect(
      commandTouchesCombat(
        {
          seq: 1,
          world_id: WORLD,
          author: "gm",
          ts: 0,
          ops: [
            {
              op: "update",
              doc_id: "tok-1",
              changes: [{ path: "/embedded/effects/0/engine/active", old: true, new: false }],
            },
          ],
        },
        store,
      ),
    ).toBe(true);
  });
});

describe("CombatHookEmitter", () => {
  it("two synchronous emit calls with an awaiting listener observe strict order", async () => {
    const hooks = new HookBus(silentLogger);
    defineCombatHooks(hooks);
    const seen: number[] = [];
    hooks.on("combat:round-start", async (payload) => {
      await Promise.resolve();
      seen.push((payload as { round: number }).round);
    });
    const emitter = new CombatHookEmitter(hooks, silentLogger);
    emitter.emit([{ name: "combat:round-start", payload: { combatId: "c", round: 1 } }]);
    emitter.emit([{ name: "combat:round-start", payload: { combatId: "c", round: 2 } }]);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(seen).toEqual([1, 2]);
  });

  it("a throwing listener does not stall the queue", async () => {
    const hooks = new HookBus(silentLogger);
    defineCombatHooks(hooks);
    hooks.on("combat:round-start", () => {
      throw new Error("boom");
    });
    const seen: number[] = [];
    hooks.on("combat:round-end", (payload) => {
      seen.push((payload as { round: number }).round);
    });
    const emitter = new CombatHookEmitter(hooks, silentLogger);
    emitter.emit([
      { name: "combat:round-start", payload: { combatId: "c", round: 1 } },
      { name: "combat:round-end", payload: { combatId: "c", round: 1 } },
    ]);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    expect(seen).toEqual([1]);
  });
});
