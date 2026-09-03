import { describe, test, expect } from "vitest";
import { buildTableDoc, TABLE_DOC_TYPE } from "./table-docs";
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
    const doc = buildTableDoc("w1", "Loot", engine, "t1");
    expect(doc.id).toBe("t1");
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
