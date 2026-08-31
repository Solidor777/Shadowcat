import { test, expect, it, vi } from "vitest";
import { DocumentStore, AssetResolver, buildActorDoc, buildTokenFromActor, buildFactionRegistryDoc, buildConditionRegistryDoc, buildSceneDoc, buildTokenDoc } from "@shadowcat/core";
import { MockBackend, TokenView } from "./index";
import type { WireDocument, WireOperation, FootprintLookup } from "@shadowcat/core";

function tokenDoc(id: string, x: number, y: number, asset: string): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: null,
    engine: { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset }, actor_id: null, overrides: null, face: null },
    system: {},
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("a dragging token snaps to its new position; a non-dragging one tweens", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  // Mark dragging: the local dragger must follow the pointer with no tween lag.
  view.setDragging("t1");
  store.applyCommand(cmd(2, [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 0, new: 100 }] }]));
  view.reconcile();
  expect(backend.tokens.get("t1")!.x).toBe(100); // snapped immediately
  // Clear dragging: a subsequent move tweens (current lags behind target).
  view.setDragging(null);
  store.applyCommand(cmd(3, [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 100, new: 200 }] }]));
  view.reconcile();
  expect(backend.tokens.get("t1")!.x).toBeLessThan(200);
});

test("reconcile creates a token node at its center transform with the resolved url", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 100, 50, "img1") }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: assets.url("img1") }, borderColor: null, badges: [], shape: "square" });
});

test("a moved token tweens via tick toward the new position", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "update", doc_id: "t1", changes: [{ path: "/engine/x", old: 0, new: 100 }] }]));
  view.reconcile(); // sets the new target; current still ~0 (existing token, not snapped)
  expect(backend.tokens.get("t1")!.x).toBeLessThan(100);
  view.tick(10_000); // settle
  expect(backend.tokens.get("t1")!.x).toBe(100);
});

test("renders a linked token using the actor's visual", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 10, y: 20 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({ kind: "image", url: assets.url("actorimg") });
});

test("a deleted token doc removes its node", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  expect(backend.tokens.has("t1")).toBe(false);
});

test("resolves the faction border color from the registry", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const registry = buildFactionRegistryDoc("w1", { f1: { name: "F1", color: "#ff0000", stance: "hostile" } }, "reg1");
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: "f1", conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: registry }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.borderColor).toBe(0xff0000);
});

test("a token with no faction has a null border", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act2",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok2");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok2")!.borderColor).toBeNull();
});

test("resolves condition icon glyphs into token badges via the registry", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" }, prone: { name: "Prone", icon: "🛌" } }, "creg1");
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: ["dead", "prone"], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: registry }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok1")!.badges).toEqual(["💀", "🛌"]);
});

test("a token whose actor has no conditions has empty badges", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act2",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok2");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok2")!.badges).toEqual([]);
});

test("reconciles a token to the server's resolved extent, with the shape still read from the actor", () => {
  // The store carries a 2x2 actor on a 100-unit square grid — the inputs a size formula would
  // multiply into 200x200 — while the wire states 173.2x200 (a hex extent). The pushed spec
  // reports the wire's numbers, so a formula reintroduced on this side fails here.
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc(
    "w1",
    "Ogre",
    { displayName: "Ogre", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 2 }, shape: "circle", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: scene }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  const footprints: FootprintLookup = { token: (id) => (id === "tok1" ? { w: 173.2, h: 200 } : null), unit: () => null };
  new TokenView(store, assets, backend, () => null, () => footprints).reconcile();
  const spec = backend.tokens.get("tok1")!;
  expect(spec.w).toBe(173.2);
  expect(spec.h).toBe(200);
  expect(spec.shape).toBe("circle");
});

test("a token the server has stated no extent for keeps its document's own authored extent", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc(
    "w1",
    "Ogre",
    { displayName: "Ogre", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 2 }, shape: "circle", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: scene }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  const spec = backend.tokens.get("tok1")!;
  expect(spec.w).toBe(100);
  expect(spec.h).toBe(100);
});

test("raw token keeps its own size + defaults to square", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const token = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 80, h: 80, rotation: 0, visual: { kind: "image", asset: "a1" }, actor_id: null, overrides: null, face: null, elevation: null }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  const spec = backend.tokens.get("tok1")!;
  expect(spec.w).toBe(80);
  expect(spec.h).toBe(80);
  expect(spec.shape).toBe("square");
});

test("renders an animated frame-list visual with resolved frame URLs", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Wisp",
    { displayName: "Wisp", visual: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 6, loop: true }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "animated",
    source: { type: "frames", urls: [assets.url("f1"), assets.url("f2")] },
    fps: 6,
    loop: true,
  });
});

test("renders an animated grid-sheet visual with a resolved sheet URL", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Torch",
    { displayName: "Torch", visual: { kind: "animated", source: { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: 7 }, fps: 12, loop: false }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "animated",
    source: { type: "sheet", url: assets.url("sheet1"), rows: 2, cols: 4, count: 7 },
    fps: 12,
    loop: false,
  });
});

test("renders an animated grid-sheet visual with a null count coalesced to undefined", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Torch",
    { displayName: "Torch", visual: { kind: "animated", source: { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: null }, fps: 12, loop: false }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "animated",
    source: { type: "sheet", url: assets.url("sheet1"), rows: 2, cols: 4, count: undefined },
    fps: 12,
    loop: false,
  });
});

test("a token whose visual fails to resolve (empty faces) is skipped, not crashed", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Broken",
    { displayName: "Broken", visual: { kind: "faces", faces: {}, default: "x", faceMap: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  expect(() => new TokenView(store, new AssetResolver(), backend).reconcile()).not.toThrow();
  expect(backend.tokens.has("tok1")).toBe(false);
});

test("tick() forwards dtMs to the backend's tickTokenAnimations", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 });
  const backend = new MockBackend();
  const spy = vi.spyOn(backend, "tickTokenAnimations");
  const view = new TokenView(store, new AssetResolver(), backend);
  view.reconcile();
  view.tick(16);
  expect(spy).toHaveBeenCalledWith(16);
});

// ---- helpers for animation-config tests ----

/** Extends MockBackend with convenience accessors for token position queries. */
class RecordingBackend extends MockBackend {
  lastTokenX(id: string): number {
    return this.tokens.get(id)!.x;
  }
  lastTokenY(id: string): number {
    return this.tokens.get(id)!.y;
  }
}

/** Build a DocumentStore pre-seeded with a single token at the given position. */
function makeStoreWithToken(id: string, pos: { x: number; y: number }): DocumentStore {
  const store = new DocumentStore();
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc(id, pos.x, pos.y, "img1") }]));
  return store;
}

/** Apply an authoritative move to a token already in the store. */
function moveToken(store: DocumentStore, id: string, pos: { x: number; y: number }): void {
  const prev = (store.query("token").find((d) => d.id === id)?.engine as { x: number; y: number }) ?? { x: 0, y: 0 };
  store.applyCommand(
    cmd(
      (store.query("token").find((d) => d.id === id)?.updated_at ?? 0) + 2,
      [
        { op: "update", doc_id: id, changes: [
          { path: "/engine/x", old: prev.x, new: pos.x },
          { path: "/engine/y", old: prev.y, new: pos.y },
        ]},
      ],
    ),
  );
}

// Animation config reaches the animator: a slow speed makes a move take longer.
it("setAnimationConfig + setWorldUnitsPerCell drive tween duration", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 });
  const backend = new RecordingBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setWorldUnitsPerCell(100);
  view.setAnimationConfig({ speedCellsPerSec: 1, easing: "linear" }); // 1 cell/s
  view.reconcile(); // snap at (0,0)
  moveToken(store, "tok1", { x: 100, y: 0 }); // 1 cell → 1000ms
  view.reconcile();
  view.tick(500); // half → ~x=50
  expect(backend.lastTokenX("tok1")).toBeCloseTo(50, 0);
  view.tick(500);
  expect(backend.lastTokenX("tok1")).toBeCloseTo(100, 0);
});

it("animateAlongPath walks the route polyline", () => {
  const store = makeStoreWithToken("tok1", { x: 0, y: 0 });
  const backend = new RecordingBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setWorldUnitsPerCell(100);
  view.setAnimationConfig({ speedCellsPerSec: 6, easing: "linear" });
  view.reconcile();
  view.animateAlongPath("tok1", [[0, 0], [300, 0], [300, 300]]); // 6 cells → 1000ms
  view.tick(500);
  expect(backend.lastTokenX("tok1")).toBeCloseTo(300, 0); // at the corner
  expect(backend.lastTokenY("tok1")).toBeCloseTo(0, 0);
});
