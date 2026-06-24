import { expect, test } from "vitest";
import { topTokenAt } from "./hit-test";
import { buildSceneDoc, buildActorDoc, buildTokenFromActor, buildTokenDoc } from "@shadowcat/core";
import type { ReadableDocuments, WireDocument } from "@shadowcat/core";

function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return { get: (id) => docs.find((d) => d.id === id), query: (type) => docs.filter((d) => d.doc_type === type), subscribe: () => () => {}, appliedSeq: 0 } as ReadableDocuments;
}
const actorSys = (over = {}) => ({ name: "G", displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, ...over });

test("circle token: a point in the corner of its bounding box misses", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 0, y: 0 }, store)).toBe("tok1");   // center: hit
  expect(topTokenAt([token], { x: 48, y: 48 }, store)).toBeNull();   // corner of the 100px box: miss
});

test("multi-cell square token is picked across its full footprint", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ size: { w: 3, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 140, y: 0 }, store)).toBe("tok1"); // inside 300px box, outside a 1-cell box
});

test("raw token uses its own box; topmost (last) wins on overlap", () => {
  const a = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" } }, "a");
  const b = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" } }, "b");
  expect(topTokenAt([a, b], { x: 0, y: 0 }, fakeStore([a, b]))).toBe("b");
});
