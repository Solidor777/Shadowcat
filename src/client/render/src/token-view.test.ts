import { test, expect, it, vi } from "vitest";
import { DocumentStore, AssetResolver, buildActorDoc, buildTokenFromActor, buildFactionRegistryDoc, buildConditionRegistryDoc, buildSceneDoc, buildTokenDoc, EMPTY_FOOTPRINTS } from "@shadowcat/core";
import { MockBackend, TokenView } from "./index";
import type { WireDocument, WireOperation, FootprintLookup, TokenVisual } from "@shadowcat/core";

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
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, visual: { kind: "image", url: assets.url("img1") }, borderColor: null, badges: [], shape: "square", perceived: false });
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
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
  const registry = buildFactionRegistryDoc("w1", { f1: { name: "F1", color: "#ff0000", stance: "hostile", movement: [] } }, "reg1");
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: "f1", conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: ["dead", "prone"], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act2",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok2");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok2")!.badges).toEqual([]);
});

/** A raw token doc at ground with the given stored elevation (raw = what the wire carries). */
function elevatedToken(id: string, elevation: number | null): WireDocument {
  const doc = tokenDoc(id, 0, 0, "img1");
  (doc.engine as { elevation?: number | null }).elevation = elevation;
  return doc;
}

test("a token off the ground plane gains an elevation chip after its condition glyphs", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [
    { op: "create", doc: elevatedToken("t-up", 10) },
    { op: "create", doc: elevatedToken("t-down", -2.5) },
    { op: "create", doc: elevatedToken("t-noisy", 0.30000000000000004) },
  ]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("t-up")!.badges).toEqual(["↑10"]);
  expect(backend.tokens.get("t-down")!.badges).toEqual(["↓2.5"]);
  // Float noise prints clean (two-decimal display rounding), not the raw f64.
  expect(backend.tokens.get("t-noisy")!.badges).toEqual(["↑0.3"]);
});

test("grounded (0 / absent / non-finite stored) tokens get no elevation chip", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [
    { op: "create", doc: elevatedToken("t-zero", 0) },
    { op: "create", doc: elevatedToken("t-absent", null) },
    { op: "create", doc: elevatedToken("t-nan", NaN) },
  ]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("t-zero")!.badges).toEqual([]);
  expect(backend.tokens.get("t-absent")!.badges).toEqual([]);
  expect(backend.tokens.get("t-nan")!.badges).toEqual([]);
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
    { displayName: "Ogre", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 2 }, shape: "circle", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "Ogre", visual: { kind: "image", asset: "a1" }, size: { w: 2, h: 2 }, shape: "circle", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "Wisp", visual: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 6, loop: true }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "Torch", visual: { kind: "animated", source: { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: 7 }, fps: 12, loop: false }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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
    { displayName: "Torch", visual: { kind: "animated", source: { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: null }, fps: 12, loop: false }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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

test("renders a generated visual with resolved art URL and parsed frame colors", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Framed",
    { displayName: "Framed", visual: { kind: "generated", art: { kind: "image", asset: "portrait" }, crop: "circle", border: { color: "#ff8800", width: 0.06 }, background: { color: "#102030" } }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "generated",
    art: { kind: "image", url: assets.url("portrait") },
    crop: "circle",
    border: { color: 0xff8800, width: 0.06 },
    background: { color: 0x102030 },
  });
});

test("renders a generated visual with animated art and no authored frame fields", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Framed Wisp",
    { displayName: "Framed Wisp", visual: { kind: "generated", art: { kind: "animated", source: { type: "frames", frames: ["f1", "f2"] }, fps: 6, loop: true }, crop: "square", border: null, background: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "generated",
    art: { kind: "animated", source: { type: "frames", urls: [assets.url("f1"), assets.url("f2")] }, fps: 6, loop: true },
    crop: "square",
    border: undefined,
    background: undefined,
  });
});

test("renders a generated face selected from a faces visual", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Many",
    { displayName: "Many", visual: { kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, framed: { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: null, background: null } }, default: "framed", faceMap: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.visual).toEqual({
    kind: "generated",
    art: { kind: "image", url: assets.url("p1") },
    crop: "circle",
    border: undefined,
    background: undefined,
  });
});

test("a generated visual with nested generated art is skipped, not crashed", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const nested: TokenVisual = { kind: "generated", art: { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: null, background: null }, crop: "circle", border: null, background: null };
  const actor = buildActorDoc(
    "w1",
    "Nested",
    { displayName: "Nested", visual: nested, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  expect(() => new TokenView(store, new AssetResolver(), backend).reconcile()).not.toThrow();
  expect(backend.tokens.has("tok1")).toBe(false);
});

test("a token whose visual fails to resolve (empty faces) is skipped, not crashed", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "Broken",
    { displayName: "Broken", visual: { kind: "faces", faces: {}, default: "x", faceMap: null }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
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

// ---- aura emission resolution ----

/** Build a linked actor+token pair in a fresh store; the actor carries the given aura (or none). */
function storeWithAuraToken(aura: { color: string; opacity: number; radius: number; enabled: boolean } | null, override?: { color: string; opacity: number; radius: number; enabled: boolean } | null): { store: DocumentStore; backend: MockBackend } {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  if (override !== undefined) {
    (token.engine as { overrides?: unknown }).overrides = { name: null, visual: null, size: null, shape: null, vision: null, aura: override, sound: null, vfx: null };
  }
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  return { store, backend };
}

test("an enabled aura resolves to a spec disc: color packed, radius converted via the view's cell-size source, opacity clamped", () => {
  const { store, backend } = storeWithAuraToken({ color: "#ffcc66", opacity: 1.5, radius: 2, enabled: true });
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setWorldUnitsPerCell(70);
  view.reconcile();
  expect(backend.tokens.get("tok1")!.aura).toEqual({ color: 0xffcc66, opacity: 1, radius: 140 });
});

test("a disabled or zero-radius aura, and a raw token, render no disc", () => {
  for (const aura of [
    { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: false },
    { color: "#ffcc66", opacity: 0.4, radius: 0, enabled: true },
    null,
  ]) {
    const { store, backend } = storeWithAuraToken(aura);
    new TokenView(store, new AssetResolver(), backend).reconcile();
    expect(backend.tokens.get("tok1")!.aura).toBeUndefined();
  }
  const rawStore = new DocumentStore();
  const rawBackend = new MockBackend();
  rawStore.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  new TokenView(rawStore, new AssetResolver(), rawBackend).reconcile();
  expect(rawBackend.tokens.get("t1")!.aura).toBeUndefined();
});

test("a per-token aura override replaces the actor's base aura wholesale", () => {
  const { store, backend } = storeWithAuraToken(
    { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true },
    { color: "#0000ff", opacity: 0.9, radius: 1, enabled: true },
  );
  const view = new TokenView(store, new AssetResolver(), backend);
  view.setWorldUnitsPerCell(100);
  view.reconcile();
  expect(backend.tokens.get("tok1")!.aura).toEqual({ color: 0x0000ff, opacity: 0.9, radius: 100 });
});

// ---- condition fx + selection highlight resolution ----

/** Build a linked actor+token pair in a fresh store, with a condition registry whose entries
 * carry the given fx payloads; the actor carries `conditionIds` in that order. */
function storeWithFxToken(
  registry: Parameters<typeof buildConditionRegistryDoc>[1],
  conditionIds: string[],
): { store: DocumentStore; backend: MockBackend } {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const reg = buildConditionRegistryDoc("w1", registry, "creg1");
  const actor = buildActorDoc(
    "w1",
    "G",
    { displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: conditionIds, prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: reg }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  return { store, backend };
}

test("condition fx folds into the TokenNodeSpec in condition array order, colors packed, at the condition strength", () => {
  const { store, backend } = storeWithFxToken(
    {
      poisoned: { name: "Poisoned", icon: "🤢", fx: { tint: "#66ff66" } },
      blinded: { name: "Blinded", icon: "🙈", fx: { desaturate: true } },
      hasted: { name: "Hasted", icon: "⚡", fx: { highlight: "#ffee00" } },
    },
    ["poisoned", "blinded", "hasted"],
  );
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok1")!.fx).toEqual([
    { kind: "tint", color: 0x66ff66, strength: 0.5 },
    { kind: "desaturate" },
    { kind: "highlight", color: 0xffee00, strength: 0.5 },
  ]);
});

test("one condition's fx fields fold tint → desaturate → highlight", () => {
  const { store, backend } = storeWithFxToken(
    { cursed: { name: "Cursed", icon: "🕸", fx: { highlight: "#ffffff", desaturate: true, tint: "#102030" } } },
    ["cursed"],
  );
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok1")!.fx).toEqual([
    { kind: "tint", color: 0x102030, strength: 0.5 },
    { kind: "desaturate" },
    { kind: "highlight", color: 0xffffff, strength: 0.5 },
  ]);
});

test("a malformed fx color contributes no entry (fail closed), and no conditions means no fx", () => {
  const { store, backend } = storeWithFxToken(
    { broken: { name: "Broken", icon: "🕸", fx: { tint: "green", highlight: "#fff" } } },
    ["broken"],
  );
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok1")!.fx).toBeUndefined();
  const plain = storeWithFxToken({ dead: { name: "Dead", icon: "💀" } }, ["dead"]);
  new TokenView(plain.store, new AssetResolver(), plain.backend).reconcile();
  expect(plain.backend.tokens.get("tok1")!.fx).toBeUndefined();
});

test("a selected token's spec appends the selection highlight after every condition fx", () => {
  const { store, backend } = storeWithFxToken(
    { poisoned: { name: "Poisoned", icon: "🤢", fx: { tint: "#66ff66" } } },
    ["poisoned"],
  );
  const selected = new Set<string>(["tok1"]);
  new TokenView(store, new AssetResolver(), backend, () => null, undefined, undefined, () => selected).reconcile();
  expect(backend.tokens.get("tok1")!.fx).toEqual([
    { kind: "tint", color: 0x66ff66, strength: 0.5 },
    { kind: "highlight", color: 0xffd400, strength: 0.4 },
  ]);
});

test("an unselected token (or a view with no selection source) gains no highlight", () => {
  const { store, backend } = storeWithFxToken({ dead: { name: "Dead", icon: "💀" } }, ["dead"]);
  const selected = new Set<string>(["someone-else"]);
  new TokenView(store, new AssetResolver(), backend, () => null, undefined, undefined, () => selected).reconcile();
  expect(backend.tokens.get("tok1")!.fx).toBeUndefined();
  // Selection is re-read per reconcile: selecting the token and re-reconciling adds the highlight.
  selected.add("tok1");
  const view = new TokenView(store, new AssetResolver(), backend, () => null, undefined, undefined, () => selected);
  view.reconcile();
  expect(backend.tokens.get("tok1")!.fx).toEqual([{ kind: "highlight", color: 0xffd400, strength: 0.4 }]);
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

test("the perceived lookup flags matching tokens; leaving the set restores the flag on the next reconcile", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  let perceived: ReadonlySet<string> = new Set(["t1"]);
  const view = new TokenView(store, new AssetResolver(), backend, () => null, () => EMPTY_FOOTPRINTS, () => perceived);
  store.applyCommand(cmd(1, [
    { op: "create", doc: tokenDoc("t1", 0, 0, "img1") },
    { op: "create", doc: tokenDoc("t2", 0, 0, "img1") },
  ]));
  view.reconcile();
  expect(backend.tokens.get("t1")!.perceived).toBe(true);
  expect(backend.tokens.get("t2")!.perceived).toBe(false);

  perceived = new Set();
  view.reconcile();
  expect(backend.tokens.get("t1")!.perceived).toBe(false);
});
