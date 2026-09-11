// @vitest-environment node
import { describe, it, expect } from "vitest";
import { buildNoteDoc } from "@shadowcat/core";
import { buildNoteTree } from "./tree";

describe("buildNoteTree", () => {
  it("orders root siblings by (engine.sort, created_at)", () => {
    const a = buildNoteDoc("w1", "A", "", { id: "a", sort: 1 });
    a.created_at = 0;
    const b = buildNoteDoc("w1", "B", "", { id: "b", sort: 0 });
    b.created_at = 1;
    const tree = buildNoteTree([a, b]);
    expect(tree.map((n) => n.doc.id)).toEqual(["b", "a"]);
  });

  it("nests children under their parent, ordered the same way", () => {
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p" });
    const childB = buildNoteDoc("w1", "B", "", { id: "cb", parentId: "p", sort: 1 });
    const childA = buildNoteDoc("w1", "A", "", { id: "ca", parentId: "p", sort: 0 });
    const tree = buildNoteTree([parent, childB, childA]);
    expect(tree).toHaveLength(1);
    expect(tree[0].doc.id).toBe("p");
    expect(tree[0].children.map((n) => n.doc.id)).toEqual(["ca", "cb"]);
  });

  it("promotes a child to root when its parent is absent from the recipient's view", () => {
    const orphan = buildNoteDoc("w1", "Orphan", "", { id: "o", parentId: "hidden-parent" });
    const tree = buildNoteTree([orphan]);
    expect(tree.map((n) => n.doc.id)).toEqual(["o"]);
  });

  it("treats a null parent_id as root", () => {
    const root = buildNoteDoc("w1", "Root", "");
    const tree = buildNoteTree([root]);
    expect(tree.map((n) => n.doc.id)).toEqual([root.id]);
  });

  it("renders a two-note mutual cycle as one root and its child", () => {
    const a = buildNoteDoc("w1", "A", "", { id: "a", parentId: "b" });
    a.created_at = 0;
    const b = buildNoteDoc("w1", "B", "", { id: "b", parentId: "a" });
    b.created_at = 1;
    const tree = buildNoteTree([a, b]);
    expect(tree.map((n) => n.doc.id)).toEqual(["a"]);
    expect(tree[0].children.map((n) => n.doc.id)).toEqual(["b"]);
    expect(tree[0].children[0].children).toEqual([]);
  });

  it("renders a self-referencing note as a root with no children", () => {
    const self = buildNoteDoc("w1", "Self", "", { id: "s", parentId: "s" });
    const tree = buildNoteTree([self]);
    expect(tree.map((n) => n.doc.id)).toEqual(["s"]);
    expect(tree[0].children).toEqual([]);
  });

  it("interleaves a cycle-promoted root among ordinary roots by (engine.sort, created_at)", () => {
    const before = buildNoteDoc("w1", "Before", "", { id: "a", sort: 0 });
    const after = buildNoteDoc("w1", "After", "", { id: "c2", sort: 2 });
    const cycleA = buildNoteDoc("w1", "CycleA", "", { id: "x", parentId: "y", sort: 1 });
    cycleA.created_at = 0;
    const cycleB = buildNoteDoc("w1", "CycleB", "", { id: "y", parentId: "x" });
    cycleB.created_at = 1;
    const tree = buildNoteTree([before, after, cycleA, cycleB]);
    expect(tree.map((n) => n.doc.id)).toEqual(["a", "x", "c2"]);
  });

  it("renders every note exactly once when a cycle coexists with a normal tree", () => {
    const root = buildNoteDoc("w1", "Root", "", { id: "r" });
    const child = buildNoteDoc("w1", "Child", "", { id: "c", parentId: "r" });
    const cycleA = buildNoteDoc("w1", "CycleA", "", { id: "x", parentId: "y" });
    cycleA.created_at = 0;
    const cycleB = buildNoteDoc("w1", "CycleB", "", { id: "y", parentId: "x" });
    cycleB.created_at = 1;
    const notes = [root, child, cycleA, cycleB];
    const tree = buildNoteTree(notes);

    const flatten = (nodes: typeof tree): string[] =>
      nodes.flatMap((n) => [n.doc.id, ...flatten(n.children)]);
    const ids = flatten(tree);
    expect(ids.sort()).toEqual(["c", "r", "x", "y"].sort());
    expect(new Set(ids).size).toBe(notes.length);
  });
});
