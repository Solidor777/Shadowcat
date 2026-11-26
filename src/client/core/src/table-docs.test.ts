import { describe, test, expect } from "vitest";
import { buildTableDoc, buildTableFolderDoc, TABLE_DOC_TYPE, TABLE_FOLDER_DOC_TYPE } from "./table-docs";
import { AUTHOR_CAPS } from "./scene-docs";
import type { TableEngine } from "@shadowcat/types";

const engine: TableEngine = {
  draw: { kind: "weighted" },
  rows: [{ weight: 1, range: null, label: "a sword", results: [] }],
  description: "",
};

describe("buildTableDoc", () => {
  test("builds a standalone, observer-default table document", () => {
    const doc = buildTableDoc("w1", "Loot", engine);
    expect(doc.doc_type).toBe(TABLE_DOC_TYPE);
    expect(doc.name).toBe("Loot");
    expect(doc.parent_id).toBeNull();
    expect(doc.engine).toEqual(engine);
    expect(doc.system).toEqual({});
    expect(doc.permissions.default).toBe("observer");
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  });

  test("uses the explicit id when given", () => {
    const doc = buildTableDoc("w1", "Loot", engine, { id: "t1" });
    expect(doc.id).toBe("t1");
  });

  test("grants the author Owner plus AUTHOR_CAPS when opts.owner is given", () => {
    const doc = buildTableDoc("w1", "Loot", engine, { owner: "gm-1" });
    expect(doc.permissions.users).toEqual({ "gm-1": "owner" });
    for (const cap of AUTHOR_CAPS) {
      expect(doc.permissions.capabilities.by_role.owner).toContain(cap);
    }
  });

  test("files the table under opts.parentId", () => {
    expect(buildTableDoc("w1", "Loot", engine, { parentId: "f1" }).parent_id).toBe("f1");
  });

  test("a Formula table's RowRange bounds are plain numbers that survive JSON.stringify", () => {
    // RowRange.lo/hi are i32 (not i64), so ts-rs emits `number`, not `bigint` --
    // a `bigint` literal here would throw inside WsClient.send's JSON.stringify
    // before the write ever reached the wire.
    const formulaEngine: TableEngine = {
      draw: { kind: "formula", notation: "2d6" },
      rows: [
        { weight: 1, range: { lo: 2, hi: 6 }, label: "low", results: [] },
        { weight: 1, range: { lo: 7, hi: 12 }, label: "high", results: [] },
      ],
      description: "",
    };
    const doc = buildTableDoc("w1", "Ranged", formulaEngine);
    expect(() => JSON.stringify(doc)).not.toThrow();
    const roundTripped = JSON.parse(JSON.stringify(doc));
    expect(roundTripped.engine).toEqual(formulaEngine);
  });
});

describe("buildTableFolderDoc", () => {
  test("builds a folder envelope carrying only the sort key", () => {
    const doc = buildTableFolderDoc("w1", "Loot", { id: "f1", parentId: "f0", sort: 2000 });
    expect(doc.doc_type).toBe(TABLE_FOLDER_DOC_TYPE);
    expect(doc.id).toBe("f1");
    expect(doc.name).toBe("Loot");
    expect(doc.parent_id).toBe("f0");
    expect(doc.engine).toEqual({ sort: 2000 });
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  });

  test("defaults to a root folder with sort 0", () => {
    const doc = buildTableFolderDoc("w1", "Loot");
    expect(doc.parent_id).toBeNull();
    expect(doc.engine).toEqual({ sort: 0 });
  });

  test("grants the author Owner plus AUTHOR_CAPS when opts.owner is given", () => {
    const doc = buildTableFolderDoc("w1", "Loot", { owner: "gm-1" });
    expect(doc.permissions.users).toEqual({ "gm-1": "owner" });
    for (const cap of AUTHOR_CAPS) {
      expect(doc.permissions.capabilities.by_role.owner).toContain(cap);
    }
  });

  test("the envelope survives JSON.stringify", () => {
    expect(() => JSON.stringify(buildTableFolderDoc("w1", "Loot"))).not.toThrow();
  });
});
