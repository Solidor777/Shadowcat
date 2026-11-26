import { describe, it, expect } from "vitest";
import {
  DocumentStore,
  buildActorDoc,
  buildTokenFromActor,
  parseBars,
  EMPTY_BARS,
  type BarAttribute,
  type TokenEngine,
  type WireDocument,
} from "@shadowcat/core";
import { resolveQuickBars } from "./barQuickEdit";

const leafBar: BarAttribute = { label: "HP", current: "hp.value", max: "hp.max", color: "#cc2222", mode: "always" };
const computedBar: BarAttribute = { label: "Temp", current: "hp.value + 2", max: 10, color: "#2222cc", mode: "always" };
const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null as string | null, conditions: [], prototype: false,
  vision: null, light: null, movement: [], aura: null, sound: null, vfx: null,
  bars: [leafBar, computedBar],
};

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A `"bars"` channel payload delivering `bars` for token `tok1`. */
function delivered(bars: { label: string; current: number; max: number; color: string; mode: string }[]) {
  return parseBars({ scenes: { scene1: { tokens: { tok1: { nameplate: "G", nameplate_mode: "always", bars } } } } });
}

const hpDelivered = { label: "HP", current: 7, max: 10, color: "#cc2222", mode: "always" };
const tempDelivered = { label: "Temp", current: 9, max: 10, color: "#2222cc", mode: "always" };

describe("resolveQuickBars", () => {
  it("targets the linked actor's system leaf for a bare-reference bar and none for a computed one", () => {
    const actor = buildActorDoc("w1", "G", actorEngine, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const rows = resolveQuickBars(token, storeWith(actor, token), delivered([hpDelivered, tempDelivered]));
    expect(rows.map((r) => [r.label, r.current, r.max])).toEqual([["HP", 7, 10], ["Temp", 9, 10]]);
    expect(rows[0].target).toEqual({ doc: expect.objectContaining({ id: "act1" }), path: "/system/hp/value" });
    expect(rows[1].target).toBeNull();
  });

  it("targets the token's embedded actor copy for an instanced token", () => {
    const actor = buildActorDoc("w1", "G", actorEngine, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const rows = resolveQuickBars(token, storeWith(token), delivered([hpDelivered]));
    expect(rows[0].target).toEqual({ doc: expect.objectContaining({ id: "tok1" }), path: "/embedded/actor/0/system/hp/value" });
  });

  it("reads a linked token's own bar list when it overrides the actor's", () => {
    const actor = buildActorDoc("w1", "G", actorEngine, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    (token.engine as TokenEngine).overrides = {
      name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null,
      bars: [{ ...leafBar, current: "stamina.now" }],
    };
    const rows = resolveQuickBars(token, storeWith(actor, token), delivered([hpDelivered]));
    expect(rows[0].target?.path).toBe("/system/stamina/now");
  });

  it("matches each delivered bar to the authored bar it came from when an earlier authored bar is not delivered", () => {
    const actor = buildActorDoc("w1", "G", { ...actorEngine, bars: [{ ...computedBar, label: "Hidden", mode: "none" as const }, leafBar] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const rows = resolveQuickBars(token, storeWith(actor, token), delivered([hpDelivered]));
    expect(rows).toHaveLength(1);
    expect(rows[0].target?.path).toBe("/system/hp/value");
  });

  it("builds a nested pointer from every dotted segment of the reference", () => {
    const actor = buildActorDoc("w1", "G", { ...actorEngine, bars: [{ ...leafBar, current: "a_b.c" }] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveQuickBars(token, storeWith(actor, token), delivered([hpDelivered]))[0].target?.path).toBe("/system/a_b/c");
  });

  it("returns nothing for a token the channel states nothing for, a dangling link, or a raw token", () => {
    const actor = buildActorDoc("w1", "G", actorEngine, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveQuickBars(token, storeWith(actor, token), EMPTY_BARS)).toEqual([]);
    expect(resolveQuickBars(token, storeWith(token), delivered([hpDelivered]))).toEqual([]);
    const raw = { ...token, engine: { ...(token.engine as TokenEngine), actor_id: null } } as WireDocument;
    expect(resolveQuickBars(raw, storeWith(raw), delivered([hpDelivered]))).toEqual([]);
  });

  it("gives a delivered bar with no authored match no target", () => {
    const actor = buildActorDoc("w1", "G", actorEngine, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const rows = resolveQuickBars(token, storeWith(actor, token), delivered([{ ...hpDelivered, label: "Stranger" }]));
    expect(rows[0].target).toBeNull();
  });
});
