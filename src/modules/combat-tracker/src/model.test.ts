import { describe, it, expect } from "vitest";
import { rowsFor, moveInOrder, rollTargets, formatResource, type Row } from "./model";
import { buildCombatantDoc, buildCombatantGroupDoc, type CombatantEngine, type WireDocument, type CombatsView } from "@shadowcat/core";

function actorCombatant(id: string, opts: { initiative?: number | null; owner?: string | null; tokenId?: string; hidden?: boolean } = {}): WireDocument {
  const engine: CombatantEngine = {
    kind: { type: "actor", token_id: opts.tokenId ?? null, actor_id: null },
    initiative: opts.initiative ?? null,
    tiebreak: 0, defeated: false, group: null,
    resources: {},
  };
  return buildCombatantDoc("w1", "combat-1", engine, { id, owner: opts.owner, hidden: opts.hidden, name: id });
}

function eventCombatant(id: string, lifespan: number | null = 1): WireDocument {
  const engine: CombatantEngine = { kind: { type: "event", lifespan, message: null }, initiative: null, tiebreak: 0, defeated: false, group: null, resources: {} };
  return buildCombatantDoc("w1", "combat-1", engine, { id, name: "An event" });
}

describe("rowsFor", () => {
  it("joins combatant docs with their resolved view, keeping input order", () => {
    const a = actorCombatant("a");
    const b = actorCombatant("b");
    const resolved: CombatsView = {
      combats: [{ id: "combat-1", sceneId: "s1", combatants: [{ id: "b", resources: null, movementCells: null }] }],
    };
    const rows = rowsFor([a, b], resolved);
    expect(rows.map((r) => r.doc.id)).toEqual(["a", "b"]);
    expect(rows[0].view).toBeNull();
    expect(rows[1].view).toEqual({ id: "b", resources: null, movementCells: null });
  });

  it("tags actor and event rows by kind", () => {
    const actor = actorCombatant("a");
    const event = eventCombatant("e");
    const rows = rowsFor([actor, event], { combats: [] });
    expect(rows[0].kind).toBe("actor");
    expect(rows[1].kind).toBe("event");
  });

  it("carries the token/actor id for art resolution on an actor row, and nothing for an event", () => {
    const actor = actorCombatant("a", { tokenId: "token-1" });
    const event = eventCombatant("e");
    const rows = rowsFor([actor, event], { combats: [] });
    expect(rows[0].art).toEqual({ tokenId: "token-1" });
    expect(rows[1].art).toEqual({});
  });

  it("resolves an empty rows list for an empty combatants list", () => {
    expect(rowsFor([], { combats: [] })).toEqual([]);
  });
});

describe("moveInOrder", () => {
  it("produces a permutation of the same id set", () => {
    const order = ["a", "b", "c", "d"];
    const moved = moveInOrder(order, 0, 2);
    expect(moved).toEqual(["b", "c", "a", "d"]);
    expect([...moved].sort()).toEqual([...order].sort());
  });

  it("moving backward shifts the intervening elements forward", () => {
    expect(moveInOrder(["a", "b", "c", "d"], 3, 1)).toEqual(["a", "d", "b", "c"]);
  });

  it("is a no-op in VALUE when from equals to, but never returns the same array reference", () => {
    const order = ["a", "b", "c"];
    const moved = moveInOrder(order, 1, 1);
    expect(moved).toEqual(order);
    expect(moved).not.toBe(order);
  });

  it("throws RangeError on an out-of-range index", () => {
    expect(() => moveInOrder(["a", "b"], -1, 0)).toThrow(RangeError);
    expect(() => moveInOrder(["a", "b"], 0, 5)).toThrow(RangeError);
  });
});

describe("rollTargets", () => {
  const rows = (docs: WireDocument[]): Row[] => rowsFor(docs, { combats: [] });

  it("GM: every actor row with initiative === null, events excluded", () => {
    const withInit = actorCombatant("has-init", { initiative: 5 });
    const noInit = actorCombatant("no-init", { initiative: null });
    const event = eventCombatant("e");
    const targets = rollTargets(rows([withInit, noInit, event]), "gm", "user-1");
    expect(targets).toEqual(["no-init"]);
  });

  it("player: own rows only, even among rows with no initiative", () => {
    const mine = actorCombatant("mine", { initiative: null, owner: "user-1" });
    const theirs = actorCombatant("theirs", { initiative: null, owner: "user-2" });
    const targets = rollTargets(rows([mine, theirs]), "player", "user-1");
    expect(targets).toEqual(["mine"]);
  });

  it("player: an own row that already has an initiative is excluded", () => {
    const mine = actorCombatant("mine", { initiative: 3, owner: "user-1" });
    const targets = rollTargets(rows([mine]), "player", "user-1");
    expect(targets).toEqual([]);
  });

  const inGroup = (id: string, group: string, owner: string): WireDocument => {
    const doc = actorCombatant(id, { owner });
    (doc.engine as CombatantEngine).group = group;
    return doc;
  };

  it("GM: a group collapses to one target, its group id, at the group's first row", () => {
    const docs = [inGroup("a", "g1", "u2"), actorCombatant("solo"), inGroup("b", "g1", "u2")];
    expect(rollTargets(rows(docs), "gm", "gm-1")).toEqual(["g1", "solo"]);
  });

  it("player: a group is a target only when every member is theirs", () => {
    const allMine = [inGroup("a", "g1", "user-1"), inGroup("b", "g1", "user-1")];
    expect(rollTargets(rows(allMine), "player", "user-1")).toEqual(["g1"]);
    const mixed = [inGroup("a", "g1", "user-1"), inGroup("b", "g1", "user-2")];
    expect(rollTargets(rows(mixed), "player", "user-1")).toEqual([]);
  });

  it("a group whose members all have an initiative is not a target", () => {
    const a = inGroup("a", "g1", "user-1");
    (a.engine as CombatantEngine).initiative = 4;
    expect(rollTargets(rows([a]), "gm", "gm-1")).toEqual([]);
  });
});

describe("rowsFor groups", () => {
  it("joins a combatant to its group's name and color, and leaves an unknown group null", () => {
    const a = actorCombatant("a");
    (a.engine as CombatantEngine).group = "g1";
    const b = actorCombatant("b");
    (b.engine as CombatantEngine).group = "ghost";
    const g = buildCombatantGroupDoc("w1", "combat-1", "Wolves", "#112233", "g1");
    const rows = rowsFor([a, b, actorCombatant("c")], { combats: [] }, [g]);
    expect(rows[0].group).toEqual({ id: "g1", name: "Wolves", color: "#112233" });
    expect(rows[1].group).toBeNull();
    expect(rows[2].group).toBeNull();
  });
});

describe("formatResource", () => {
  it("renders a tracked resource as current / max", () => {
    expect(formatResource({ binding: "tracked", current: 12, max: 12, error: null })).toBe("12 / 12");
    expect(formatResource({ binding: "tracked", current: 7, max: 30, error: null })).toBe("7 / 30");
  });

  it("renders a mirror resource as its current value alone", () => {
    expect(formatResource({ binding: "mirror", current: 4, max: null, error: null })).toBe("4");
  });

  it("renders the warning glyph on an evaluation error", () => {
    expect(formatResource({ binding: "tracked", current: null, max: null, error: "bad formula" })).toBe("⚠");
  });

  it("renders an em dash for an undefined view", () => {
    expect(formatResource(undefined)).toBe("—");
  });
});
