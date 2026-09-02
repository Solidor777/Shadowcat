import { describe, it, expect } from "vitest";
import { envelope, type WireDocument } from "@shadowcat/core";
import {
  folderChildren,
  folderPathNames,
  buildMoveOp,
  isDescendantOrSelf,
  buildFolderDoc,
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
});

describe("folderPathNames", () => {
  it("walks root-to-leaf", () => {
    expect(folderPathNames(DOCS, "grand")).toEqual(["alpha", "zulu", "golf"]);
  });
});

describe("buildMoveOp", () => {
  it("carries the target and the true OCC pre-image", () => {
    expect(buildMoveOp("child-z", null, "root-a")).toEqual({
      op: "move",
      doc_id: "child-z",
      parent_id: null,
      old_parent_id: "root-a",
    });
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
  it("builds an asset_folder envelope with the engine sort", () => {
    const d = buildFolderDoc("w1", "maps", "root-a");
    expect(d.doc_type).toBe("asset_folder");
    expect(d.name).toBe("maps");
    expect(d.parent_id).toBe("root-a");
    expect(d.engine).toEqual({ sort: 0 });
  });
});
