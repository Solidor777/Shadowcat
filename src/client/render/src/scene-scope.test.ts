import { describe, it, expect } from "vitest";
import { DocumentStore, buildTokenDoc, buildSceneDoc, buildSceneEntityDoc } from "@shadowcat/core";
import { sceneScopedDocs } from "./scene-scope";

function store(): DocumentStore {
  const s = new DocumentStore();
  const mk = (id: string, scene: string) => buildTokenDoc("w1", scene, { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null }, id);
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
    { op: "create", doc: mk("t-a", "sA") },
    { op: "create", doc: mk("t-b", "sB") },
  ] });
  return s;
}

describe("sceneScopedDocs", () => {
  it("returns only the viewed scene's children", () => {
    const s = store();
    expect(sceneScopedDocs(s, "token", () => "sA").map((d) => d.id)).toEqual(["t-a"]);
    expect(sceneScopedDocs(s, "token", () => "sB").map((d) => d.id)).toEqual(["t-b"]);
  });
  it("returns ALL of the type when no scene is viewed (degenerate)", () => {
    expect(sceneScopedDocs(store(), "token", () => null).map((d) => d.id).sort()).toEqual(["t-a", "t-b"]);
  });
});

describe("sceneScopedDocs level scoping", () => {
  function levelStore(): DocumentStore {
    const s = new DocumentStore();
    const sceneDoc = buildSceneDoc(
      "w1",
      {
        levels: [
          { id: "l1", name: "Ground", bottom: 0, top: 10, background: null },
          { id: "l2", name: "Upper", bottom: 10, top: 20, background: null },
        ],
      },
      "sA",
    );
    const mkToken = (id: string, elevation: number | null) =>
      buildTokenDoc(
        "w1",
        "sA",
        { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation },
        id,
      );
    const mkWall = (id: string, elevation: { bottom: number | null; top: number | null } | null) =>
      buildSceneEntityDoc(
        "w1",
        "sA",
        "wall",
        { seg: { x1: 0, y1: 0, x2: 1, y2: 1 }, blocksSight: true, blocksMove: true, blocksLight: true, elevation },
        id,
      );
    s.applyCommand({
      seq: 1,
      world_id: "w1",
      author: "u",
      ts: 0,
      ops: [
        { op: "create", doc: sceneDoc },
        { op: "create", doc: mkToken("t-ground", 0) },
        { op: "create", doc: mkToken("t-upper", 15) },
        { op: "create", doc: mkWall("w-ground", { bottom: 0, top: 9 }) },
        { op: "create", doc: mkWall("w-upper", { bottom: 10, top: 20 }) },
      ],
    });
    return s;
  }

  it("scopes a point-elevation doc type (token) via levelOf", () => {
    const s = levelStore();
    expect(sceneScopedDocs(s, "token", () => "sA", () => "l1").map((d) => d.id)).toEqual(["t-ground"]);
    expect(sceneScopedDocs(s, "token", () => "sA", () => "l2").map((d) => d.id)).toEqual(["t-upper"]);
  });

  it("scopes a band-shaped doc type (wall) via bandContains at the level's bottom", () => {
    const s = levelStore();
    expect(sceneScopedDocs(s, "wall", () => "sA", () => "l1").map((d) => d.id)).toEqual(["w-ground"]);
    expect(sceneScopedDocs(s, "wall", () => "sA", () => "l2").map((d) => d.id)).toEqual(["w-upper"]);
  });

  it("preserves today's behavior exactly when viewedLevel resolves to null", () => {
    const s = levelStore();
    expect(sceneScopedDocs(s, "token", () => "sA", () => null).map((d) => d.id).sort()).toEqual(["t-ground", "t-upper"]);
    expect(sceneScopedDocs(s, "token", () => "sA").map((d) => d.id).sort()).toEqual(["t-ground", "t-upper"]);
  });

  it("preserves today's behavior for a scene with no levels", () => {
    const s = store();
    expect(sceneScopedDocs(s, "token", () => "sA", () => "l1").map((d) => d.id)).toEqual(["t-a"]);
  });
});
