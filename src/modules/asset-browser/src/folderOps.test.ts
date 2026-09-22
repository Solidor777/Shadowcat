// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { envelope, type WireDocument } from "@shadowcat/core";
import {
  folderChildren,
  folderPathNames,
  isDescendantOrSelf,
  buildFolderDoc,
  appendFolderSortKey,
} from "./folderOps";

function folder(id: string, name: string, parent: string | null, sort = 0): WireDocument {
  const d = envelope("w1", "asset_folder", parent, {}, id, { sort }, name);
  return d;
}

const DOCS = [
  folder("root-b", "beta", null, 1),
  folder("root-a", "alpha", null, 0),
  folder("child-z", "zulu", "root-a"),
  folder("child-m", "mike", "root-a"),
  folder("grand", "golf", "child-z"),
];

describe("folderChildren", () => {
  it("filters by parent and orders by sort then name", () => {
    expect(folderChildren(DOCS, null).map((d) => d.id)).toEqual(["root-a", "root-b"]);
    expect(folderChildren(DOCS, "root-a").map((d) => d.id)).toEqual(["child-m", "child-z"]);
  });

  it("uses the injected name comparator for the tie-break, never its own", () => {
    // A reversed comparator flips the name tie-break within an equal-sort group.
    const reversed = (a: string, b: string) => b.localeCompare(a);
    expect(folderChildren(DOCS, "root-a", reversed).map((d) => d.id)).toEqual(["child-z", "child-m"]);
  });
});

describe("folderPathNames", () => {
  it("walks root-to-leaf", () => {
    expect(folderPathNames(DOCS, "grand")).toEqual(["alpha", "zulu", "golf"]);
  });
});


describe("isDescendantOrSelf", () => {
  it("covers self, deep descent, and unrelated nodes", () => {
    expect(isDescendantOrSelf(DOCS, "root-a", "root-a")).toBe(true);
    expect(isDescendantOrSelf(DOCS, "root-a", "grand")).toBe(true);
    expect(isDescendantOrSelf(DOCS, "root-b", "grand")).toBe(false);
  });
});

describe("buildFolderDoc", () => {
  it("builds an asset_folder envelope with the engine sort, defaulting to 0", () => {
    const d = buildFolderDoc("w1", "maps", "root-a");
    expect(d.doc_type).toBe("asset_folder");
    expect(d.name).toBe("maps");
    expect(d.parent_id).toBe("root-a");
    expect(d.engine).toEqual({ sort: 0 });
  });

  it("accepts an explicit sort key", () => {
    const d = buildFolderDoc("w1", "maps", "root-a", 2500);
    expect(d.engine).toEqual({ sort: 2500 });
  });
});

describe("appendFolderSortKey", () => {
  it("allocates the density constant for the first child of an empty parent", () => {
    expect(appendFolderSortKey([], "root-a")).toBe(1000);
  });

  it("allocates one density step past the highest existing sibling", () => {
    // root-a's children (child-m sort 0, child-z sort 0) both default to 0 in the fixture.
    expect(appendFolderSortKey(DOCS, "root-a")).toBe(1000);
    // root level: root-a sort 0, root-b sort 1 — highest is 1.
    expect(appendFolderSortKey(DOCS, null)).toBe(1001);
  });
});
