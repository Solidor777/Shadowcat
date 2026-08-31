import { expect, test } from "vitest";
import { topTokenAt } from "./hit-test";
import { buildSceneDoc, buildActorDoc, buildTokenFromActor, buildTokenDoc, EMPTY_FOOTPRINTS } from "@shadowcat/core";
import type { ReadableDocuments, WireDocument, FootprintLookup } from "@shadowcat/core";

function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return { get: (id) => docs.find((d) => d.id === id), query: (type) => docs.filter((d) => d.doc_type === type), subscribe: () => () => {}, appliedSeq: 0 } as ReadableDocuments;
}
const actorEngine = (over = {}) => ({ displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, light: null, ...over });

/** A lookup stating one token's server-resolved extent, standing in for a `"footprints"` frame. */
function footprintsFor(tokenId: string, w: number, h: number): FootprintLookup {
  return { token: (id) => (id === tokenId ? { w, h } : null), unit: () => null };
}

test("circle token: a point in the corner of its bounding box misses", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "G", actorEngine({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const store = fakeStore([scene, actor, token]);
  const fp = footprintsFor("tok1", 100, 100);
  expect(topTokenAt([token], { x: 0, y: 0 }, store, fp)).toBe("tok1");   // center: hit
  expect(topTokenAt([token], { x: 48, y: 48 }, store, fp)).toBeNull();   // corner of the 100px box: miss
});

test("multi-cell square token is picked across its full footprint", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "G", actorEngine({ size: { w: 3, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 140, y: 0 }, store, footprintsFor("tok1", 300, 300))).toBe("tok1"); // inside the 300px extent, outside a one-cell one
});

test("a hex token is picked over the hex it occupies, wider and taller than one cell size", () => {
  // A 1-hex token on a circumradius-100 hex grid occupies a hex spanning √3·100 ≈ 173.2 across
  // the flats and 200 point to point, so its half-extents are ≈86.6 and 100. A point at x=80 is
  // inside that hex and outside the 100x100 square a square-sized token would be picked over;
  // a point at x=95 is outside both, so the widened extent does not simply pick everything.
  const scene = buildSceneDoc("w1", { grid: { kind: "hex", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "G", actorEngine(), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100 * Math.sqrt(3), h: 200 }, "tok1");
  const store = fakeStore([scene, actor, token]);
  const fp = footprintsFor("tok1", 100 * Math.sqrt(3), 200);
  expect(topTokenAt([token], { x: 80, y: 0 }, store, fp)).toBe("tok1");
  expect(topTokenAt([token], { x: 95, y: 0 }, store, fp)).toBeNull();
  // The hex is taller than it is wide: y=95 hits where x=95 misses.
  expect(topTokenAt([token], { x: 0, y: 95 }, store, fp)).toBe("tok1");
});

test("a hex token the server has stated no extent for is picked over its own authored extent", () => {
  // The optimistic window before the token's own resolved extent arrives: the placement path
  // stamped the scene's unit footprint, and that is what picks.
  const scene = buildSceneDoc("w1", { grid: { kind: "hex", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "G", actorEngine(), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100 * Math.sqrt(3), h: 200 }, "tok1");
  const store = fakeStore([scene, actor, token]);
  expect(topTokenAt([token], { x: 80, y: 0 }, store, EMPTY_FOOTPRINTS)).toBe("tok1");
  expect(topTokenAt([token], { x: 95, y: 0 }, store, EMPTY_FOOTPRINTS)).toBeNull();
});

test("raw token uses its own box; topmost (last) wins on overlap", () => {
  const a = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" }, actor_id: null, overrides: null, face: null, elevation: null }, "a");
  const b = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "x" }, actor_id: null, overrides: null, face: null, elevation: null }, "b");
  expect(topTokenAt([a, b], { x: 0, y: 0 }, fakeStore([a, b]), EMPTY_FOOTPRINTS)).toBe("b");
});
