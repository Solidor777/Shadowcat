import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";
import { DocumentStore, type WireDocument, type WireOperation, type EffectEngine } from "@shadowcat/core";
import EffectsList from "./EffectsList.svelte";
import { listEffects, effectOriginText, type EffectLookup } from "./effectsList";
import { setAppContextForTest } from "./__fixtures__/appContextTest";

function doc(id: string, docType: string, over: Partial<WireDocument> = {}): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: docType, schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: null, engine: undefined, system: {}, created_at: 0, updated_at: 0,
    ...over,
  };
}

function effect(id: string, name: string | null, engine: Partial<EffectEngine> = {}): WireDocument {
  return doc(id, "effect", { name, engine: { active: true, transfer: false, duration: null, ...engine } });
}

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d }) as WireOperation) });
  return s;
}

const none: EffectLookup = { get: () => undefined, query: () => [] };

describe("effectOriginText", () => {
  it("prefers the label, then the resolved source name, else nothing", () => {
    const lookup: EffectLookup = { get: (id) => (id === "src" ? doc("src", "item", { name: "Spell scroll" }) : undefined), query: () => [] };
    expect(effectOriginText({ source: "src", label: "Aldric" }, lookup)).toBe("Aldric");
    expect(effectOriginText({ source: "src" }, lookup)).toBe("Spell scroll");
    expect(effectOriginText({ source: "gone" }, lookup)).toBeNull();
    expect(effectOriginText(null, lookup)).toBeNull();
  });
});

describe("listEffects", () => {
  it("lists an actor's own effects and only the transferring item effects", () => {
    const actor = doc("a1", "actor", {
      embedded: {
        effect: [effect("e1", "Bless", { origin: { label: "Aldric" } })],
        item: [doc("i1", "item", { embedded: { effect: [effect("e2", "Ring glow", { transfer: true }), effect("e3", "Cursed", { transfer: false })] } })],
      },
    });
    expect(listEffects(actor, none, "Effect")).toEqual([
      { id: "e1", name: "Bless", active: true, origin: "Aldric" },
      { id: "e2", name: "Ring glow", active: true, origin: null },
    ]);
  });

  it("names an effect's origin after the embedded item it came from when the origin has no label", () => {
    const ring = doc("i1", "item", { name: "Ring of Warding", embedded: { effect: [effect("e2", "Ring glow", { transfer: true, origin: { source: "i1" } })] } });
    const actor = doc("a1", "actor", { embedded: { effect: [effect("e1", "Ward", { origin: { source: "i1" } })], item: [ring] } });
    expect(listEffects(actor, none, "Effect").map((e) => e.origin)).toEqual(["Ring of Warding", "Ring of Warding"]);
    const labelled = doc("a2", "actor", { embedded: { effect: [effect("e3", "Ward", { origin: { source: "i1", label: "Aldric" } })], item: [ring] } });
    expect(listEffects(labelled, none, "Effect")[0].origin).toBe("Aldric");
  });

  it("reads a token through its embedded actor copy and labels an unnamed effect", () => {
    const token = doc("t1", "token", { embedded: { actor: [doc("a1", "actor", { embedded: { effect: [effect("e1", null, { active: false })] } })] } });
    expect(listEffects(token, none, "Effect")).toEqual([{ id: "e1", name: "Effect", active: false, origin: null }]);
    expect(listEffects(doc("t2", "token"), none, "Effect")).toEqual([]);
  });

  it("with an anchor, drops own effects anchored elsewhere and adds effects anchored to it", () => {
    const duration = (anchor: string | null) => ({ amount: { type: "number", value: 1 }, unit: "rounds", anchor, expires: "round_end" }) as unknown as EffectEngine["duration"];
    const host = doc("a1", "actor", { embedded: { effect: [effect("own", "Own", { duration: duration(null) }), effect("away", "Away", { duration: duration("c2") })] } });
    const other = doc("a2", "actor", { embedded: { effect: [effect("remote", "Remote", { duration: duration("c1") }), effect("unrelated", "Unrelated", { duration: duration(null) })] } });
    const lookup: EffectLookup = { get: () => undefined, query: (t) => (t === "actor" ? [host, other] : []) };
    expect(listEffects(host, lookup, "Effect", "c1").map((e) => e.id)).toEqual(["own", "remote"]);
  });
});

describe("EffectsList", () => {
  it("renders each effect with its origin and marks an inactive one", () => {
    const host = doc("a1", "actor", { embedded: { effect: [effect("e1", "Bless", { origin: { label: "Aldric" } }), effect("e2", "Poison", { active: false })] } });
    const context = setAppContextForTest({ documents: storeWith(host), t: (k, p) => (p ? `${k}:${p.origin}` : k) });
    const { getByTestId, queryByText } = render(EffectsList, { props: { host }, context });
    expect(getByTestId("effects-list:e1").textContent).toContain("effects.origin:Aldric");
    expect(getByTestId("effects-list:e2").textContent).toContain("effects.inactive");
    expect(queryByText("effects.origin:undefined")).toBeNull();
  });

  it("follows the live store, so an effect added after mount appears", async () => {
    const host = doc("a1", "actor");
    const documents = storeWith(host);
    const context = setAppContextForTest({ documents });
    const { queryByTestId, findByTestId } = render(EffectsList, { props: { host }, context });
    expect(queryByTestId("effects-list")).toBeNull();
    const withEffect = doc("a1", "actor", { embedded: { effect: [effect("e9", "Haste")] } });
    documents.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "delete", doc: host } as WireOperation, { op: "create", doc: withEffect } as WireOperation] });
    expect((await findByTestId("effects-list:e9")).textContent).toContain("Haste");
  });

  it("renders nothing without a host or effects", () => {
    const context = setAppContextForTest({ documents: storeWith() });
    expect(render(EffectsList, { props: { host: undefined }, context }).queryByTestId("effects-list")).toBeNull();
  });
});
