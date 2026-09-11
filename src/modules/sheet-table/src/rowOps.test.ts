// @vitest-environment node
import { describe, it, expect } from "vitest";
import { addRow, removeRow, moveRow, setRow, defaultEntry, normalizeRowsForDraw } from "./rowOps";
import type { TableRow } from "@shadowcat/core";

function row(label: string): TableRow {
  return { weight: 1, range: null, label, results: [] };
}

describe("addRow", () => {
  it("appends a weighted row with a null range", () => {
    const next = addRow([row("a")], { kind: "weighted" });
    expect(next).toHaveLength(2);
    expect(next[1]).toEqual({ weight: 1, range: null, label: "", results: [] });
    expect(next[0]).toEqual(row("a")); // unchanged
  });

  it("appends a formula row with a placeholder range", () => {
    const next = addRow([], { kind: "formula", notation: "1d20" });
    expect(next[0].range).toEqual({ lo: 1, hi: 1 });
  });

  it("never mutates the input array", () => {
    const rows = [row("a")];
    addRow(rows, { kind: "weighted" });
    expect(rows).toHaveLength(1);
  });
});

describe("removeRow", () => {
  it("removes the row at index", () => {
    const next = removeRow([row("a"), row("b")], 0);
    expect(next).toEqual([row("b")]);
  });

  it("throws on an out-of-range index", () => {
    expect(() => removeRow([row("a")], 5)).toThrow(RangeError);
  });
});

describe("moveRow", () => {
  it("moves a row up", () => {
    const next = moveRow([row("a"), row("b"), row("c")], 2, -1);
    expect(next.map((r) => r.label)).toEqual(["a", "c", "b"]);
  });

  it("moves a row down", () => {
    const next = moveRow([row("a"), row("b"), row("c")], 0, 1);
    expect(next.map((r) => r.label)).toEqual(["b", "a", "c"]);
  });

  it("clamps at the top boundary (no-op)", () => {
    const rows = [row("a"), row("b")];
    const next = moveRow(rows, 0, -1);
    expect(next.map((r) => r.label)).toEqual(["a", "b"]);
    expect(next).not.toBe(rows);
  });

  it("clamps at the bottom boundary (no-op)", () => {
    const next = moveRow([row("a"), row("b")], 1, 1);
    expect(next.map((r) => r.label)).toEqual(["a", "b"]);
  });
});

describe("setRow", () => {
  it("replaces the row at index", () => {
    const next = setRow([row("a"), row("b")], 1, row("z"));
    expect(next.map((r) => r.label)).toEqual(["a", "z"]);
  });

  it("throws on an out-of-range index", () => {
    expect(() => setRow([row("a")], 3, row("z"))).toThrow(RangeError);
  });
});

describe("normalizeRowsForDraw", () => {
  it("clears every row's range when switching to weighted", () => {
    const rows: TableRow[] = [
      { weight: 1, range: { lo: 1, hi: 10 }, label: "a", results: [] },
      { weight: 1, range: { lo: 11, hi: 20 }, label: "b", results: [] },
    ];
    const next = normalizeRowsForDraw(rows, { kind: "weighted" });
    expect(next.every((r) => r.range === null)).toBe(true);
  });

  it("preserves an existing valid range when switching to formula", () => {
    const rows: TableRow[] = [{ weight: 1, range: { lo: 5, hi: 9 }, label: "a", results: [] }];
    const next = normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(next[0].range).toEqual({ lo: 5, hi: 9 });
  });

  it("seeds the addRow placeholder range for a null-range row switching to formula", () => {
    const rows: TableRow[] = [{ weight: 1, range: null, label: "a", results: [] }];
    const next = normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(next[0].range).toEqual({ lo: 1, hi: 1 });
  });

  it("never mutates the input array or its rows", () => {
    const rows: TableRow[] = [{ weight: 1, range: null, label: "a", results: [] }];
    normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(rows[0].range).toBeNull();
  });
});

describe("defaultEntry", () => {
  it("builds a default text entry", () => {
    expect(defaultEntry("text")).toEqual({ kind: "text", text: "" });
  });

  it("builds a default doc entry", () => {
    expect(defaultEntry("doc")).toEqual({ kind: "doc", target: { kind: "doc", doc_id: "", embedded_path: null }, label: "" });
  });

  it("builds a default image entry", () => {
    expect(defaultEntry("image")).toEqual({ kind: "image", asset_id: "", alt: "" });
  });

  it("builds a default draw entry", () => {
    expect(defaultEntry("draw")).toEqual({ kind: "draw", table_id: "", count: 1 });
  });
});
