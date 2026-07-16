import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import { ContributionRegistry } from "./contributions";
import { envelope } from "./scene-docs";
import { resolveDocRef, pickSheet, sheetContract, SHEET_FALLBACK_CONTRACT } from "./sheets";
import type { WireDocument } from "./wire";

function store(docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("resolveDocRef", () => {
  it("resolves a top-level docId to itself, /system write site", () => {
    const doc = envelope("w1", "actor", null, { name: "A" }, "d1");
    const t = resolveDocRef({ docId: "d1" }, store([doc]));
    expect(t).toEqual({ panelId: "sheet:d1", doc, writeDocId: "d1", writePrefix: "/system" });
  });

  it("resolves a linked token to its actor doc, /system write site", () => {
    const actor = envelope("w1", "actor", null, { name: "Goblin" }, "a1");
    // `actor_id` is engine-owned (TokenEngine) — the token's `system` stays empty here.
    const token = envelope("w1", "token", "sc1", {}, "t1", { actor_id: "a1", overrides: null });
    const t = resolveDocRef({ tokenId: "t1" }, store([actor, token]));
    expect(t?.panelId).toBe("sheet:a1");
    expect(t?.writeDocId).toBe("a1");
    expect(t?.writePrefix).toBe("/system");
    expect((t?.doc.system as { name: string }).name).toBe("Goblin");
  });

  it("resolves an instanced token to its embedded actor, /embedded/actor/0/system write site", () => {
    const embedded = envelope("w1", "actor", null, { name: "Copy" }, "e1");
    const token = envelope("w1", "token", "sc1", { x: 0, y: 0 }, "t2");
    token.embedded = { actor: [embedded] };
    const t = resolveDocRef({ tokenId: "t2" }, store([token]));
    expect(t?.panelId).toBe("sheet:t2/embedded/actor/0");
    expect(t?.writeDocId).toBe("t2");
    expect(t?.writePrefix).toBe("/embedded/actor/0/system");
    expect((t?.doc.system as { name: string }).name).toBe("Copy");
  });

  it("round-trips the instanced-token panelId through the embedded-child docId form", () => {
    const embedded = envelope("w1", "actor", null, { name: "Copy" }, "e1");
    const token = envelope("w1", "token", "sc1", { x: 0, y: 0 }, "t2");
    token.embedded = { actor: [embedded] };
    const s = store([token]);
    const fromToken = resolveDocRef({ tokenId: "t2" }, s);
    const fromDocId = resolveDocRef({ docId: "t2", embeddedPath: "/embedded/actor/0" }, s);
    expect(fromDocId?.writeDocId).toBe(fromToken?.writeDocId);
    expect(fromDocId?.writePrefix).toBe(fromToken?.writePrefix);
    expect(fromDocId?.panelId).toBe(fromToken?.panelId);
  });

  it("resolves a one-level embedded child ref, /embedded/<coll>/<idx>/system write site", () => {
    const item = envelope("w1", "item", null, { name: "Sword" }, "i1");
    const actor = envelope("w1", "actor", null, { name: "A" }, "a2");
    actor.embedded = { item: [item] };
    const t = resolveDocRef({ docId: "a2", embeddedPath: "/embedded/item/0" }, store([actor]));
    expect(t?.panelId).toBe("sheet:a2/embedded/item/0");
    expect(t?.writeDocId).toBe("a2");
    expect(t?.writePrefix).toBe("/embedded/item/0/system");
    expect((t?.doc.system as { name: string }).name).toBe("Sword");
  });

  it("fails closed on a dangling link, a raw token, a missing doc, and a bad embedded index", () => {
    const linked = envelope("w1", "token", "sc1", {}, "t3", { actor_id: "gone" });
    const raw = envelope("w1", "token", "sc1", {}, "t4", { x: 0, y: 0 });
    const s = store([linked, raw]);
    expect(resolveDocRef({ tokenId: "t3" }, s)).toBeNull();
    expect(resolveDocRef({ tokenId: "t4" }, s)).toBeNull();
    expect(resolveDocRef({ docId: "nope" }, s)).toBeNull();
    expect(resolveDocRef({ docId: "t4", embeddedPath: "/embedded/actor/9" }, s)).toBeNull();
  });

  it("fails closed on non-object refs from untyped runtime callers", () => {
    const s = store([]);
    expect(resolveDocRef(null as never, s)).toBeNull();
    expect(resolveDocRef(undefined as never, s)).toBeNull();
    expect(resolveDocRef("x" as never, s)).toBeNull();
  });
});

describe("pickSheet", () => {
  const doc = envelope("w1", "actor", null, {}, "d1");

  it("picks the highest-priority matching provider for the doc_type", () => {
    const reg = new ContributionRegistry();
    reg.contribute({ id: "lo", contract: sheetContract("actor"), component: "LO", sheet: { priority: 1 } });
    reg.contribute({ id: "hi", contract: sheetContract("actor"), component: "HI", sheet: { priority: 10 } });
    expect(pickSheet(reg, doc)).toBe("HI");
  });

  it("skips a provider whose match() rejects the doc", () => {
    const reg = new ContributionRegistry();
    reg.contribute({ id: "no", contract: sheetContract("actor"), component: "NO", sheet: { priority: 10, match: () => false } });
    reg.contribute({ id: "yes", contract: sheetContract("actor"), component: "YES", sheet: { priority: 1 } });
    expect(pickSheet(reg, doc)).toBe("YES");
  });

  it("tie-breaks equal priority by lowest module id", () => {
    const reg = new ContributionRegistry();
    reg.contribute({ id: "x", contract: sheetContract("actor"), component: "B", sheet: { priority: 5 } }, { module: "mod-b" });
    reg.contribute({ id: "y", contract: sheetContract("actor"), component: "A", sheet: { priority: 5 } }, { module: "mod-a" });
    expect(pickSheet(reg, doc)).toBe("A");
  });

  it("falls back to the -Infinity generic provider when no doc_type provider matches", () => {
    const reg = new ContributionRegistry();
    reg.contribute({ id: "fb", contract: SHEET_FALLBACK_CONTRACT, component: "FB", sheet: { priority: -Infinity } });
    expect(pickSheet(reg, envelope("w1", "widget", null, {}, "d9"))).toBe("FB");
  });

  it("tie-breaks two -Infinity fallbacks by module id despite NaN from subtraction", () => {
    const reg = new ContributionRegistry();
    reg.contribute(
      { id: "b", contract: SHEET_FALLBACK_CONTRACT, component: "B", sheet: { priority: -Infinity } },
      { module: "mod-b" },
    );
    reg.contribute(
      { id: "a", contract: SHEET_FALLBACK_CONTRACT, component: "A", sheet: { priority: -Infinity } },
      { module: "mod-a" },
    );
    expect(pickSheet(reg, envelope("w1", "widget", null, {}, "d9"))).toBe("A");
  });

  it("returns null when nothing (not even a fallback) is registered", () => {
    expect(pickSheet(new ContributionRegistry(), doc)).toBeNull();
  });
});
