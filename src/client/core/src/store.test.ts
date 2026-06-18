import { describe, it, expect } from "vitest";
import { DocumentStore, setPointer } from "./store";
import type { WireCommand, WireDocument } from "./wire";

function doc(id: string, system: unknown): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "actor",
    schema_version: 1,
    source: null,
    owner: null,
    permissions: {
      default: "none",
      users: {},
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
    },
    embedded: {},
    system,
    created_at: 0,
    updated_at: 0,
  };
}

function cmd(seq: number, ops: WireCommand["ops"]): WireCommand {
  return { seq, world_id: "w1", author: "a", ts: 0, ops };
}

describe("DocumentStore", () => {
  it("applies create, update, and delete", () => {
    const s = new DocumentStore();
    s.applyCommand(cmd(1, [{ op: "create", doc: doc("d1", { hp: 10 }) }]));
    expect((s.get("d1")!.system as { hp: number }).hp).toBe(10);
    expect(s.appliedSeq).toBe(1);

    s.applyCommand(
      cmd(2, [
        {
          op: "update",
          doc_id: "d1",
          changes: [{ path: "/system/hp", old: 10, new: 5 }],
        },
      ]),
    );
    expect((s.get("d1")!.system as { hp: number }).hp).toBe(5);

    s.applyCommand(cmd(3, [{ op: "delete", doc: doc("d1", {}) }]));
    expect(s.get("d1")).toBeUndefined();
    expect(s.appliedSeq).toBe(3);
  });

  it("creates intermediate objects on a nested set", () => {
    const s = new DocumentStore();
    s.applyCommand(cmd(1, [{ op: "create", doc: doc("d1", {}) }]));
    s.applyCommand(
      cmd(2, [
        {
          op: "update",
          doc_id: "d1",
          changes: [{ path: "/system/attrs/str", old: null, new: 14 }],
        },
      ]),
    );
    expect((s.get("d1")!.system as { attrs: { str: number } }).attrs.str).toBe(
      14,
    );
  });

  it("notifies and unsubscribes listeners", () => {
    const s = new DocumentStore();
    let n = 0;
    const off = s.subscribe(() => n++);
    s.applyCommand(cmd(1, [{ op: "create", doc: doc("d1", {}) }]));
    expect(n).toBe(1);
    off();
    s.applyCommand(cmd(2, [{ op: "create", doc: doc("d2", {}) }]));
    expect(n).toBe(1);
  });

  it("filters by doc_type in query", () => {
    const s = new DocumentStore();
    s.applyCommand(cmd(1, [{ op: "create", doc: doc("d1", {}) }]));
    expect(s.query("actor")).toHaveLength(1);
    expect(s.query("item")).toHaveLength(0);
  });
});

describe("setPointer", () => {
  it("rejects a pointer without a leading slash", () => {
    expect(() => setPointer({}, "system/hp", 1)).toThrow();
  });
  it("sets a nested value, creating intermediates", () => {
    const root: Record<string, unknown> = {};
    setPointer(root, "/a/b", 7);
    expect((root as { a: { b: number } }).a.b).toBe(7);
  });
});
