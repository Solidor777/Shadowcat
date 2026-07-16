import { describe, it, expect } from "vitest";
import { DocumentStore, buildTokenDoc } from "@shadowcat/core";
import { sceneScopedDocs } from "./scene-scope";

function store(): DocumentStore {
  const s = new DocumentStore();
  const mk = (id: string, scene: string) => buildTokenDoc("w1", scene, { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null }, id);
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
