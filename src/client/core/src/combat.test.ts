import { describe, it, expect, vi } from "vitest";
import { DocumentStore } from "./store";
import type { WireDocument, WireOperation, ClientMsg } from "./wire";
import {
  CombatController,
  CombatClientError,
  type CombatControllerDeps,
} from "./combat";
import { buildCombatDoc, buildCombatantDoc, newCombatEngine } from "./scene-docs";
import { silentLogger } from "./logger";
import type { CombatEngine, CombatantEngine } from "./scene-docs";

const WORLD = "world-1";

function makeToken(id: string, actorId: string | null, owner: string | null): WireDocument {
  return {
    id,
    world_id: WORLD,
    scope: null,
    doc_type: "token",
    parent_id: "scene-1",
    owner,
    name: `token-${id}`,
    engine: { x: 0, y: 0, w: 1, h: 1, rotation: 0, actor_id: actorId },
    system: {},
    embedded: {},
    permissions: { default: "observer", users: {}, property_overrides: {} },
  } as unknown as WireDocument;
}

function makeActor(id: string, owner: string | null): WireDocument {
  return {
    id,
    world_id: WORLD,
    scope: null,
    doc_type: "actor",
    parent_id: null,
    owner,
    name: `actor-${id}`,
    engine: {},
    system: {},
    embedded: {},
    permissions: { default: "observer", users: {}, property_overrides: {} },
  } as unknown as WireDocument;
}

/** A store seeded with a scene (implicit, unused directly), three tokens (two linked actors,
 * one instanced), a combat (active, order of three, turn = second), and combatants. One id in
 * `order` (0xdead) is not in the store at all -- a hidden combatant the store never received. */
function seedStore(): { store: DocumentStore; combatId: string; combatants: string[] } {
  const store = new DocumentStore();
  const combatId = "combat-1";
  const c1 = "cc-1";
  const c2 = "cc-2";
  const c3 = "cc-3";
  const hiddenId = "cc-hidden";

  const actor1 = makeActor("actor-1", "player-1");
  const token1 = makeToken("tok-1", "actor-1", "player-1");
  const token2 = makeToken("tok-2", null, "player-2"); // instanced (no actor_id link)

  const combatEngine: CombatEngine = {
    ...newCombatEngine("scene-1"),
    active: true,
    round: 1,
    turn: c2,
    order: [c1, c2, c3, "cc-ghost"],
  };
  const combatDoc = buildCombatDoc(WORLD, combatEngine, combatId);

  const cc1Engine: CombatantEngine = {
    kind: { type: "actor", token_id: "tok-1", actor_id: "actor-1" },
    initiative: 10,
    tiebreak: 0,
    resources: {},
  };
  const cc1 = buildCombatantDoc(WORLD, combatId, cc1Engine, { owner: "player-1", id: c1 });

  const cc2Engine: CombatantEngine = {
    kind: { type: "actor", token_id: "tok-2", actor_id: null },
    initiative: 15,
    tiebreak: 0,
    resources: {},
  };
  const cc2 = buildCombatantDoc(WORLD, combatId, cc2Engine, { owner: "player-2", id: c2 });

  const cc3Engine: CombatantEngine = {
    kind: { type: "event", lifespan: 2, message: null },
    initiative: 5,
    tiebreak: 0,
    resources: {},
  };
  const cc3 = buildCombatantDoc(WORLD, combatId, cc3Engine, { id: c3 });

  // A combatant parented to the combat but absent from `order` (should be appended).
  const strayEngine: CombatantEngine = {
    kind: { type: "event", lifespan: null, message: null },
    initiative: null,
    tiebreak: 0,
    resources: {},
  };
  const stray = buildCombatantDoc(WORLD, combatId, strayEngine, { id: hiddenId, name: "stray" });

  store.seedDocuments([actor1, token1, token2, combatDoc, cc1, cc2, cc3, stray]);
  return { store, combatId, combatants: [c1, c2, c3] };
}

type CombatFrame = Extract<ClientMsg, { type: `combat_${string}` }>;

function makeDeps(store: DocumentStore, overrides: Partial<CombatControllerDeps> = {}): CombatControllerDeps {
  return {
    documents: store,
    dispatchIntent: vi.fn(),
    sendCombat: vi.fn((_msg: CombatFrame) => Promise.resolve()),
    selfId: "player-1",
    role: () => "player",
    canEdit: () => true,
    world: () => WORLD,
    logger: silentLogger,
    ...overrides,
  };
}

describe("CombatController reads", () => {
  it("combatsFor / activeFor: active first, matches by scene", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store));
    const combats = combat.combatsFor("scene-1");
    expect(combats.map((d) => d.id)).toContain(combatId);
    expect(combat.activeFor("scene-1")?.id).toBe(combatId);
    expect(combat.activeFor("scene-none")).toBeNull();
  });

  it("combatants: order preserved, missing id skipped, stray parented combatant appended", () => {
    const { store, combatId, combatants } = seedStore();
    const combat = new CombatController(makeDeps(store));
    const ids = combat.combatants(combatId).map((d) => d.id);
    expect(ids.slice(0, 3)).toEqual(combatants);
    expect(ids).not.toContain("cc-ghost"); // unresolvable id skipped
    expect(ids).toContain("cc-hidden"); // stray parented combatant appended
  });

  it("turnOf: resolves the current turn, null when turn names a missing id", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store));
    expect(combat.turnOf(combatId)?.id).toBe("cc-2");

    const doc = store.get(combatId)!;
    const engine = doc.engine as CombatEngine;
    store.applyCommand({
      seq: 1,
      world_id: WORLD,
      author: "gm",
      ts: 0,
      ops: [
        {
          op: "update",
          doc_id: combatId,
          changes: [{ path: "/engine/turn", old: engine.turn, new: "cc-ghost" }],
        },
      ],
    });
    expect(combat.turnOf(combatId)).toBeNull();
  });

  it("resolvedFor / setResolved / subscribe", () => {
    const { store } = seedStore();
    const combat = new CombatController(makeDeps(store));
    expect(combat.resolvedFor("cc-1")).toBeNull();
    const listener = vi.fn();
    const unsub = combat.subscribe(listener);
    combat.setResolved({
      combats: [
        {
          id: "combat-1",
          sceneId: "scene-1",
          combatants: [{ id: "cc-1", resources: null, movementCells: null }],
        },
      ],
    });
    expect(listener).toHaveBeenCalledTimes(1);
    expect(combat.resolvedFor("cc-1")?.id).toBe("cc-1");
    unsub();
    combat.setResolved({ combats: [] });
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe("CombatController intents", () => {
  it("start/pause/end/advance/rewind/sort build the right frame and propagate rejection", async () => {
    const { store, combatId } = seedStore();
    const sendCombat = vi.fn((_msg: CombatFrame) => Promise.reject(new Error("nope")));
    const combat = new CombatController(makeDeps(store, { sendCombat }));
    await expect(combat.start(combatId)).rejects.toThrow("nope");
    const frame = sendCombat.mock.calls[0][0] as Extract<ClientMsg, { type: "combat_start" }>;
    expect(frame.type).toBe("combat_start");
    expect(frame.combat_id).toBe(combatId);
    expect(typeof frame.request_id).toBe("string");

    await expect(combat.advance(combatId)).rejects.toThrow("nope");
    await expect(combat.pause(combatId)).rejects.toThrow("nope");
    await expect(combat.end(combatId)).rejects.toThrow("nope");
    await expect(combat.rewind(combatId)).rejects.toThrow("nope");
    await expect(combat.sort(combatId)).rejects.toThrow("nope");
  });

  it("roll/modifyResource build the right frame", async () => {
    const { store, combatId } = seedStore();
    const sendCombat = vi.fn((_msg: CombatFrame) => Promise.resolve());
    const combat = new CombatController(makeDeps(store, { sendCombat }));
    await combat.roll(combatId, "table", [{ combatant_id: "cc-1", notation: "1d20" }]);
    const rollFrame = sendCombat.mock.calls[0][0] as Extract<ClientMsg, { type: "combat_roll" }>;
    expect(rollFrame.channel).toBe("table");
    expect(rollFrame.rolls).toHaveLength(1);

    await combat.modifyResource(combatId, "cc-1", "movement", { kind: "set", value: 3 });
    const resFrame = sendCombat.mock.calls[1][0] as Extract<ClientMsg, { type: "combat_resource" }>;
    expect(resFrame.combatant_id).toBe("cc-1");
    expect(resFrame.resource).toBe("movement");
  });
});

describe("CombatController document helpers", () => {
  it("createCombat builds a combat doc at the engine defaults via one intent", () => {
    const { store } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    const id = combat.createCombat("scene-2", { name: "Ambush" });
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops).toHaveLength(1);
    expect(ops[0].op).toBe("create");
    const doc = (ops[0] as { doc: WireDocument }).doc;
    expect(doc.id).toBe(id);
    expect(doc.doc_type).toBe("combat");
    expect((doc.engine as CombatEngine).active).toBe(false);
    expect(doc.name).toBe("Ambush");
  });

  it("addCombatants: one intent, creates plus one order-update with the exact old/new", () => {
    const { store, combatId } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    const before = (store.get(combatId)!.engine as CombatEngine).order;

    const ids = combat.addCombatants(combatId, [{ tokenId: "tok-1" }]);
    expect(ids).toHaveLength(1);
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops).toHaveLength(2);
    expect(ops[0].op).toBe("create");
    const created = (ops[0] as { doc: WireDocument }).doc;
    expect((created.engine as CombatantEngine).kind).toEqual({
      type: "actor",
      token_id: "tok-1",
      actor_id: "actor-1",
    });
    expect(created.owner).toBe("player-1");

    const orderOp = ops[1] as Extract<WireOperation, { op: "update" }>;
    expect(orderOp.doc_id).toBe(combatId);
    expect(orderOp.changes[0].old).toEqual(before);
    expect(orderOp.changes[0].new).toEqual([...before, ids[0]]);
  });

  it("addCombatants: actorId-only entry (no token) is admitted", () => {
    const { store, combatId } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    combat.addCombatants(combatId, [{ actorId: "actor-1" }]);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const created = (ops[0] as { doc: WireDocument }).doc;
    expect((created.engine as CombatantEngine).kind).toEqual({
      type: "actor",
      token_id: null,
      actor_id: "actor-1",
    });
  });

  it("addCombatants: no-host entry throws CombatClientError", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store));
    expect(() => combat.addCombatants(combatId, [{}])).toThrow(CombatClientError);
  });

  it("addEvent builds a one-shot event combatant and appends it to order", () => {
    const { store, combatId } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    const before = (store.get(combatId)!.engine as CombatEngine).order;
    const id = combat.addEvent(combatId, { name: "Lair action", lifespan: 3, message: "rumble" });
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const created = (ops[0] as { doc: WireDocument }).doc;
    expect(created.id).toBe(id);
    expect((created.engine as CombatantEngine).kind).toEqual({
      type: "event",
      lifespan: 3,
      message: "rumble",
    });
    const orderOp = ops[1] as Extract<WireOperation, { op: "update" }>;
    expect(orderOp.changes[0].new).toEqual([...before, id]);
  });

  it("removeCombatant: order update + delete with the store pre-image", () => {
    const { store, combatId } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    combat.removeCombatant(combatId, "cc-1");
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const orderOp = ops[0] as Extract<WireOperation, { op: "update" }>;
    expect(orderOp.changes[0].new).not.toContain("cc-1");
    const deleteOp = ops[1] as Extract<WireOperation, { op: "delete" }>;
    expect(deleteOp.op).toBe("delete");
    expect(deleteOp.doc.id).toBe("cc-1");
  });

  it("removeCombatant on the current turn throws turn-owner", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store));
    expect(() => combat.removeCombatant(combatId, "cc-2")).toThrow(CombatClientError);
  });

  it("setHidden: both directions, remove: true on the users entry when hiding", () => {
    const { store } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    combat.setHidden("cc-1", true);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const defaultOp = ops[0] as Extract<WireOperation, { op: "update" }>;
    expect(defaultOp.changes[0].new).toBe("none");
    const usersOp = ops[1] as Extract<WireOperation, { op: "update" }>;
    expect(usersOp.changes[0].remove).toBe(true);

    dispatchIntent.mockClear();
    combat.setHidden("cc-1", false);
    const ops2 = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const usersOp2 = ops2[1] as Extract<WireOperation, { op: "update" }>;
    expect(usersOp2.changes[0].new).toBe("owner");
  });

  it("reorder: set mismatch throws, matching set dispatches the update", () => {
    const { store, combatId, combatants } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    expect(() => combat.reorder(combatId, ["cc-1"])).toThrow(CombatClientError);
    const reordered = [combatants[1], combatants[0], combatants[2], "cc-ghost"];
    combat.reorder(combatId, reordered);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect((ops[0] as Extract<WireOperation, { op: "update" }>).changes[0].new).toEqual(reordered);
  });

  it("setInitiative dispatches the field update", () => {
    const { store } = seedStore();
    const dispatchIntent = vi.fn();
    const combat = new CombatController(makeDeps(store, { dispatchIntent }));
    combat.setInitiative("cc-1", 18, 2);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("update");
  });
});

describe("CombatController.canAct", () => {
  it("GM: everything true", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store, { role: () => "gm" }));
    const a = combat.canAct(combatId);
    expect(a.start).toBe(true);
    expect(a.pause).toBe(true);
    expect(a.end).toBe(true);
    expect(a.advance).toBe(true);
    expect(a.rewind).toBe(true);
    expect(a.sort).toBe(true);
    expect(a.edit).toBe(true);
    expect(a.roll("cc-2")).toBe(true);
    expect(a.resource("cc-2")).toBe(true);
  });

  it("owner under owner_may_end on own turn: advance true, GM-only flags false", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store, { role: () => "player", selfId: "player-2" }));
    const a = combat.canAct(combatId);
    expect(a.advance).toBe(true, );
    expect(a.start).toBe(false);
    expect(a.rewind).toBe(false);
  });

  it("owner under gm_only: advance false", () => {
    const { store, combatId } = seedStore();
    store.applyCommand({
      seq: 1,
      world_id: WORLD,
      author: "gm",
      ts: 0,
      ops: [
        {
          op: "update",
          doc_id: combatId,
          changes: [{ path: "/engine/turn_control", old: "owner_may_end", new: "gm_only" }],
        },
      ],
    });
    const combat = new CombatController(makeDeps(store, { role: () => "player", selfId: "player-2" }));
    expect(combat.canAct(combatId).advance).toBe(false);
  });

  it("non-owner: advance/roll/resource all false", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(makeDeps(store, { role: () => "player", selfId: "someone-else" }));
    const a = combat.canAct(combatId);
    expect(a.advance).toBe(false);
    expect(a.roll("cc-1")).toBe(false);
    expect(a.resource("cc-1")).toBe(false);
  });

  it("owner whose canEdit is false: roll/resource false, advance unaffected", () => {
    const { store, combatId } = seedStore();
    const combat = new CombatController(
      makeDeps(store, { role: () => "player", selfId: "player-1", canEdit: () => false }),
    );
    const a = combat.canAct(combatId);
    expect(a.roll("cc-1")).toBe(false);
    expect(a.resource("cc-1")).toBe(false);
  });
});
