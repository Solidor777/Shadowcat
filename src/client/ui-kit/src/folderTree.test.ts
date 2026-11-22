import { describe, it, expect } from "vitest";
import {
  typedFolderChildren,
  typedFolderSiblings,
  appendTypedFolderSortKey,
  buildTypedFolderDoc,
  isDescendantOrSelf,
} from "./folderTree";

const A = "probe_folder";
const mk = (id: string, type: string, parent: string | null, sort: number, name = id) => ({
  ...buildTypedFolderDoc("w1", type, name, parent, sort),
  id,
});

describe("typed folder helpers", () => {
  const docs = [mk("a", A, null, 2000), mk("b", A, null, 1000), mk("c", A, "a", 0), mk("x", "other_folder", null, 0)];

  it("children are filtered by doc type and parent, ordered by sort", () => {
    expect(typedFolderChildren(docs, A, null).map((d) => d.id)).toEqual(["b", "a"]);
    expect(typedFolderChildren(docs, "other_folder", null).map((d) => d.id)).toEqual(["x"]);
  });

  it("equal sort keys fall back to the name comparator", () => {
    const tied = [mk("z", A, null, 0, "Zed"), mk("y", A, null, 0, "Ace")];
    expect(typedFolderChildren(tied, A, null).map((d) => d.id)).toEqual(["y", "z"]);
  });

  it("siblings carry id and sort in display order", () => {
    expect(typedFolderSiblings(docs, A, null)).toEqual([
      { id: "b", sort: 1000 },
      { id: "a", sort: 2000 },
    ]);
  });

  it("the append key ignores other doc types", () => {
    const only = appendTypedFolderSortKey([mk("x", "other_folder", null, 9000)], A, null);
    const withOwn = appendTypedFolderSortKey(docs, A, null);
    expect(withOwn).toBeGreaterThan(2000);
    expect(only).toBeLessThan(withOwn);
  });

  it("buildTypedFolderDoc stamps the doc type, parent and sort", () => {
    const doc = buildTypedFolderDoc("w1", A, "n", "p", 5);
    expect(doc.doc_type).toBe(A);
    expect(doc.parent_id).toBe("p");
    expect(doc.engine).toEqual({ sort: 5 });
  });

  it("isDescendantOrSelf walks the parent chain", () => {
    expect(isDescendantOrSelf(docs, "a", "c")).toBe(true);
    expect(isDescendantOrSelf(docs, "a", "a")).toBe(true);
    expect(isDescendantOrSelf(docs, "c", "a")).toBe(false);
  });
});
