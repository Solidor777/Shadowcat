// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { expect, test } from "vitest";
import { topTokenAt, topRegionAt, topDrawingAt, topTemplateAt } from "./hit-test";
import { buildSceneDoc, buildActorDoc, buildTokenFromActor, buildTokenDoc, buildRegionDoc, buildSceneEntityDoc, EMPTY_FOOTPRINTS } from "@shadowcat/core";
import type { ReadableDocuments, WireDocument, FootprintLookup, RegionEngine, DrawingEngine, TemplateEngine } from "@shadowcat/core";

function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return { get: (id) => docs.find((d) => d.id === id), query: (type) => docs.filter((d) => d.doc_type === type), subscribe: () => () => {}, appliedSeq: 0 } as ReadableDocuments;
}
const actorEngine = (over = {}) => ({ displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null, ...over });

/** A lookup stating one token's server-resolved extent, standing in for a `"footprints"` frame. */
function footprintsFor(tokenId: string, w: number, h: number): FootprintLookup {
  return { token: (id) => (id === tokenId ? { w, h } : null), unit: () => null, level: () => null };
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

// --- topRegionAt ---

const regionEngine = (over: Partial<RegionEngine> = {}): RegionEngine => ({
  shape: { kind: "rect", points: [0, 0, 10, 10] },
  behavior: "terrain",
  cost: 1,
  enabled: true,
  triggers: [],
  elevation: null,
  ...over,
});

test("topRegionAt: a rect region hits inside its bbox and misses outside", () => {
  const region = buildRegionDoc("w1", "scene1", regionEngine(), "r1");
  expect(topRegionAt([region], { x: 5, y: 5 })).toBe("r1");
  expect(topRegionAt([region], { x: 50, y: 50 })).toBeNull();
});

test("topRegionAt: a circle region hits inside its radius and misses outside", () => {
  const region = buildRegionDoc("w1", "scene1", regionEngine({ shape: { kind: "circle", points: [0, 0, 5] } }), "r1");
  expect(topRegionAt([region], { x: 0, y: 0 })).toBe("r1");
  expect(topRegionAt([region], { x: 10, y: 10 })).toBeNull();
});

test("topRegionAt: overlapping regions pick the topmost (last-in-order) on containment", () => {
  const a = buildRegionDoc("w1", "scene1", regionEngine(), "a");
  const b = buildRegionDoc("w1", "scene1", regionEngine(), "b");
  expect(topRegionAt([a, b], { x: 5, y: 5 })).toBe("b");
});

test("topRegionAt: a malformed shape (bad point count) never renders or picks", () => {
  const region = buildRegionDoc("w1", "scene1", regionEngine({ shape: { kind: "rect", points: [0, 0, 10] } }), "r1");
  expect(topRegionAt([region], { x: 5, y: 5 })).toBeNull();
});

// --- topDrawingAt ---

const drawingEngine = (over: Partial<DrawingEngine> = {}): DrawingEngine => ({
  shape: { kind: "rect", points: [0, 0, 10, 10] },
  stroke: null,
  fill: null,
  elevation: null,
  ...over,
});

test("topDrawingAt: a closed rect drawing hits inside its bbox", () => {
  const drawing = buildSceneEntityDoc("w1", "scene1", "drawing", drawingEngine(), "d1");
  expect(topDrawingAt([drawing], { x: 5, y: 5 })).toBe("d1");
  expect(topDrawingAt([drawing], { x: 50, y: 50 })).toBeNull();
});

test("topDrawingAt: an open freehand line picks nearest-within-tolerance, not containment", () => {
  const drawing = buildSceneEntityDoc(
    "w1", "scene1", "drawing",
    drawingEngine({ shape: { kind: "freehand", points: [0, 0, 10, 0, 20, 0] } }),
    "d1",
  );
  expect(topDrawingAt([drawing], { x: 10, y: 2 })).toBe("d1"); // within tolerance of the polyline
  expect(topDrawingAt([drawing], { x: 10, y: 20 })).toBeNull(); // far from every segment
});

test("topDrawingAt: overlapping closed drawings pick the topmost (last-in-order) on containment", () => {
  const a = buildSceneEntityDoc("w1", "scene1", "drawing", drawingEngine(), "a");
  const b = buildSceneEntityDoc("w1", "scene1", "drawing", drawingEngine(), "b");
  expect(topDrawingAt([a, b], { x: 5, y: 5 })).toBe("b");
});

// --- topTemplateAt ---

const templateEngine = (over: Partial<TemplateEngine> = {}): TemplateEngine => ({
  shape: { kind: "circle", x: 0, y: 0, size: 10, direction: 0 },
  color: "#3388ff",
  elevation: null,
  ...over,
});

test("topTemplateAt: a circle template hits inside its radius and misses outside", () => {
  const template = buildSceneEntityDoc("w1", "scene1", "template", templateEngine(), "t1");
  expect(topTemplateAt([template], { x: 0, y: 0 })).toBe("t1");
  expect(topTemplateAt([template], { x: 50, y: 50 })).toBeNull();
});

test("topTemplateAt: an open line template picks nearest-within-tolerance, not containment", () => {
  const template = buildSceneEntityDoc(
    "w1", "scene1", "template",
    templateEngine({ shape: { kind: "line", x: 0, y: 0, size: 20, direction: 0 } }),
    "t1",
  );
  expect(topTemplateAt([template], { x: 10, y: 2 })).toBe("t1"); // within tolerance of the segment
  expect(topTemplateAt([template], { x: 10, y: 20 })).toBeNull(); // far from the segment
});

test("topTemplateAt: overlapping closed templates pick the topmost (last-in-order) on containment", () => {
  const a = buildSceneEntityDoc("w1", "scene1", "template", templateEngine(), "a");
  const b = buildSceneEntityDoc("w1", "scene1", "template", templateEngine(), "b");
  expect(topTemplateAt([a, b], { x: 0, y: 0 })).toBe("b");
});
