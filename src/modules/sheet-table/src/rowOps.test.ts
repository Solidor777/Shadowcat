// @vitest-environment node
import { describe, it, expect } from "vitest";
import { addRow, removeRow, moveRow, setRow, defaultEntry, normalizeRowsForDraw, nextFreeRange } from "./rowOps";
import type { TableRow } from "@shadowcat/core";

/** True iff every ranged row in `rows` is pairwise disjoint from every other (the
 * `TableEngine::validate` invariant `nextFreeRange` exists to satisfy). */
function pairwiseDisjoint(rows: TableRow[]): boolean {
  const ranges = rows.map((r) => r.range).filter((r): r is NonNullable<typeof r> => r !== null);
  for (let i = 0; i < ranges.length; i++) {
    if (ranges[i].lo > ranges[i].hi) return false;
    for (let j = i + 1; j < ranges.length; j++) {
      if (ranges[i].lo <= ranges[j].hi && ranges[j].lo <= ranges[i].hi) return false;
    }
  }
  return true;
}

function row(label: string): TableRow {
  return { weight: 1, range: null, label, results: [] };
}

describe("addRow", () => {
  it("appends a weighted row with a null range and the given default label", () => {
    const next = addRow([row("a")], { kind: "weighted" }, "New row");
    expect(next).toHaveLength(2);
    expect(next[1]).toEqual({ weight: 1, range: null, label: "New row", results: [] });
    expect(next[0]).toEqual(row("a")); // unchanged
  });

  it("uses the caller's default label verbatim — `addRow` has no fallback of its own", () => {
    const next = addRow([], { kind: "weighted" }, "New row");
    expect(next[0].label).toBe("New row");
  });

  it("appends a formula row with a placeholder range", () => {
    const next = addRow([], { kind: "formula", notation: "1d20" }, "New row");
    expect(next[0].range).toEqual({ lo: 1, hi: 1 });
  });

  it("appends a formula row whose range is disjoint from every existing row's range", () => {
    const rows: TableRow[] = [
      { weight: 1, range: { lo: 1, hi: 5 }, label: "a", results: [] },
      { weight: 1, range: { lo: 6, hi: 10 }, label: "b", results: [] },
    ];
    const next = addRow(rows, { kind: "formula", notation: "1d20" }, "New row");
    expect(next[2].range).toEqual({ lo: 11, hi: 11 });
    expect(pairwiseDisjoint(next)).toBe(true);
  });

  it("never mutates the input array", () => {
    const rows = [row("a")];
    addRow(rows, { kind: "weighted" }, "New row");
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

  it("seeds a placeholder range for a null-range row switching to formula", () => {
    const rows: TableRow[] = [{ weight: 1, range: null, label: "a", results: [] }];
    const next = normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(next[0].range).toEqual({ lo: 1, hi: 1 });
  });

  it("assigns pairwise-disjoint ranges to multiple rows with no range", () => {
    const rows: TableRow[] = [
      { weight: 1, range: null, label: "a", results: [] },
      { weight: 1, range: null, label: "b", results: [] },
      { weight: 1, range: null, label: "c", results: [] },
    ];
    const next = normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(pairwiseDisjoint(next)).toBe(true);
    expect(next.map((r) => r.range)).toEqual([
      { lo: 1, hi: 1 },
      { lo: 2, hi: 2 },
      { lo: 3, hi: 3 },
    ]);
  });

  it("preserves an existing range and places a missing one above the max", () => {
    const rows: TableRow[] = [
      { weight: 1, range: { lo: 5, hi: 9 }, label: "a", results: [] },
      { weight: 1, range: null, label: "b", results: [] },
    ];
    const next = normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(next[0].range).toEqual({ lo: 5, hi: 9 });
    expect(next[1].range).toEqual({ lo: 10, hi: 10 });
    expect(pairwiseDisjoint(next)).toBe(true);
  });

  it("never mutates the input array or its rows", () => {
    const rows: TableRow[] = [{ weight: 1, range: null, label: "a", results: [] }];
    normalizeRowsForDraw(rows, { kind: "formula", notation: "1d20" });
    expect(rows[0].range).toBeNull();
  });
});

describe("nextFreeRange", () => {
  it("returns {1,1} when no row carries a range", () => {
    expect(nextFreeRange([row("a"), row("b")])).toEqual({ lo: 1, hi: 1 });
  });

  it("returns the slot immediately above the max hi across ranged rows", () => {
    const rows: TableRow[] = [
      { weight: 1, range: { lo: 1, hi: 3 }, label: "a", results: [] },
      { weight: 1, range: { lo: 4, hi: 4 }, label: "b", results: [] },
    ];
    expect(nextFreeRange(rows)).toEqual({ lo: 5, hi: 5 });
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
