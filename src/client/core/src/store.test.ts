import { describe, it, expect } from "vitest";
import { DocumentStore, setPointer, removePointer, getPointer, applyOperation } from "./store";
import type { WireCommand, WireDocument } from "./wire";
import { buildSceneDoc } from "./scene-docs";

function doc(id: string, system: unknown): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "actor",
    schema_version: 1,
    name: null,
    source: null,
    owner: null,
    permissions: {
      default: "none",
      users: {},
      property_overrides: {},
      capabilities: { by_role: {}, by_user: {} },
      gm_role: null,
    },
    embedded: {},
    parent_id: null,
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

  it("applies a remove:true change as GENUINE key absence, not set-to-null", () => {
    const s = new DocumentStore();
    s.applyCommand(cmd(1, [{ op: "create", doc: doc("d1", { a: "1", b: "2" }) }]));
    s.applyCommand(
      cmd(2, [
        {
          op: "update",
          doc_id: "d1",
          changes: [{ path: "/system/a", old: "1", new: null, remove: true }],
        },
      ]),
    );
    const system = s.get("d1")!.system as Record<string, unknown>;
    expect("a" in system).toBe(false); // absent, NOT present-as-null
    expect(system.b).toBe("2"); // sibling untouched
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

  it("applies an update through an explicit null intermediate via applyOperation", () => {
    // Mirrors setPointer's null-intermediate fix through the actual update-op path
    // (applyOperation -> setPointer -> DocumentSchema.parse), not just the pointer helper.
    const docs = new Map<string, WireDocument>([["d1", doc("d1", {})]]);
    (docs.get("d1") as WireDocument & { engine: unknown }).engine = { vision: null };
    applyOperation(docs, {
      op: "update",
      doc_id: "d1",
      changes: [
        {
          path: "/engine/vision/movementRestriction",
          old: null,
          new: "revealed",
        },
      ],
    });
    expect((docs.get("d1") as WireDocument & { engine: unknown }).engine).toEqual({
      vision: { movementRestriction: "revealed" },
    });
  });

  it("commits a scene built via buildSceneDoc: set_pointer through its null vision/lighting", () => {
    // Realistic path: buildSceneDoc legitimately writes vision:null/lighting:null
    // (required-but-nullable engine keys); a GameSettingsPanel-style override write onto
    // one must commit without throwing.
    const scene = buildSceneDoc("w1", {}, "scene1");
    expect((scene.engine as { vision: unknown }).vision).toBeNull();
    expect((scene.engine as { lighting: unknown }).lighting).toBeNull();

    const s = new DocumentStore();
    s.applyCommand(cmd(1, [{ op: "create", doc: scene }]));
    expect(() =>
      s.applyCommand(
        cmd(2, [
          {
            op: "update",
            doc_id: "scene1",
            changes: [
              {
                path: "/engine/vision/movementRestriction",
                old: null,
                new: "revealed",
              },
            ],
          },
        ]),
      ),
    ).not.toThrow();
    expect(
      (s.get("scene1")!.engine as { vision: { movementRestriction: string } }).vision
        .movementRestriction,
    ).toBe("revealed");
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
  it("replaces an in-range array element", () => {
    const root = { a: [1, 2, 3] };
    setPointer(root, "/a/1", 9);
    expect(root.a).toEqual([1, 9, 3]);
  });
  it("rejects an out-of-range array index (no silent sparse extend)", () => {
    expect(() => setPointer({ a: [1, 2] }, "/a/5", 9)).toThrow();
    expect(() => setPointer({ a: [1, 2] }, "/a/x", 9)).toThrow();
  });

  // Anti-drift pin (bug: null-intermediate blocked nested set_pointer): SceneEngine's
  // vision/lighting/etc. are Option<T> with no skip_serializing_if, so they serialize as
  // explicit `null` rather than an absent key. A single fixture asserted on both this
  // client path and the server's `data::command::set_pointer` (see
  // `set_pointer_descends_through_an_explicit_null_intermediate`) pins parity: a future
  // change to one side that forgets the other must fail one of the two tests.
  const NULL_INTERMEDIATE_FIXTURE = { engine: { vision: null } };
  const NULL_INTERMEDIATE_EXPECTED = {
    engine: { vision: { movementRestriction: "revealed" } },
  };

  it("descends through an explicit null intermediate the same as a missing key", () => {
    const root = structuredClone(NULL_INTERMEDIATE_FIXTURE) as Record<string, unknown>;
    setPointer(root, "/engine/vision/movementRestriction", "revealed");
    expect(root).toEqual(NULL_INTERMEDIATE_EXPECTED);
  });

  it("descends through two nested explicit null intermediates", () => {
    const root: Record<string, unknown> = { engine: null };
    setPointer(root, "/engine/vision/mode", "dark");
    expect(root).toEqual({ engine: { vision: { mode: "dark" } } });
  });
});

describe("removePointer", () => {
  it("deletes an object key so it is genuinely absent", () => {
    const root = { a: 1, b: 2 };
    removePointer(root, "/a");
    expect("a" in root).toBe(false);
    expect(root.b).toBe(2);
  });
  it("is a no-op on an already-absent key or absent intermediate", () => {
    const root = { a: 1 };
    removePointer(root, "/missing");
    removePointer(root, "/missing/deep");
    expect(root).toEqual({ a: 1 });
  });
  it("is a no-op when removing a key beneath an explicit null intermediate (null == absent)", () => {
    // Mirrors the server remove_pointer test `remove_pointer_through_a_null_intermediate_is_a_no_op`
    // (anti-drift: a `null` intermediate has nothing beneath it, so removal is a silent no-op on
    // both sides; the `null` itself is preserved). A regression on either side fails one of the two.
    const root = { engine: { vision: null } };
    removePointer(root, "/engine/vision/mode");
    expect(root).toEqual({ engine: { vision: null } });
  });
  it("rejects array-index removal (arrays shrink via whole-array replacement)", () => {
    expect(() => removePointer({ a: [1, 2, 3] }, "/a/1")).toThrow();
  });
  it("rejects a pointer without a leading slash", () => {
    expect(() => removePointer({}, "a")).toThrow();
  });
});

describe("getPointer", () => {
  const root = { system: { name: "x", stats: { hp: 5 }, tags: ["a", "b"] } };
  it("reads nested object and array values", () => {
    expect(getPointer(root, "/system")).toEqual(root.system);
    expect(getPointer(root, "/system/stats/hp")).toBe(5);
    expect(getPointer(root, "/system/tags/1")).toBe("b");
  });
  it("returns undefined for a missing path, never throws", () => {
    expect(getPointer(root, "/system/nope/deep")).toBeUndefined();
    expect(getPointer(root, "/system/tags/9")).toBeUndefined();
    expect(getPointer(undefined, "/system")).toBeUndefined();
  });
});
