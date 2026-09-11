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
});
