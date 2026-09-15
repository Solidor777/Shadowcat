// @vitest-environment node
import { describe, expect, it } from "vitest";
import { DocumentStore, buildActorDoc, buildTokenFromActor } from "@shadowcat/core";
import type { ResolvedVfxSource, VfxEmission } from "@shadowcat/core";
import { MockBackend } from "./backend.mock";
import { VfxView, vfxAnchorZIndex } from "./vfx-view";
import type { TokenNodeSpec, TokenTransform } from "./types";

const SHEET: ResolvedVfxSource = { type: "sheet", url: "/fx.webp", rows: 2, cols: 2, count: 3 };

function vfxEmission(over: Partial<VfxEmission> = {}): VfxEmission {
  return { asset: "fx1", anchor: "token", loop: true, enabled: true, ...over };
}

/** Seeds a store with a scene-parented, actor-backed token carrying the given emission. */
function storeWithToken(vfx: VfxEmission | null, tokenId = "tok1") {
  const store = new DocumentStore();
  const actor = buildActorDoc(
    "w1",
    "G",
    {
      displayName: "G",
      visual: { kind: "image", asset: "actorimg" },
      size: { w: 1, h: 1 },
      shape: "square",
      faction: null,
      conditions: [],
      prototype: false,
      vision: null,
      light: null,
      movement: [],
      aura: null,
      sound: null,
      vfx,
    },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 10, y: 20 }, { w: 100, h: 100 }, tokenId);
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: actor }, { op: "create", doc: token }] });
  return store;
}

function tokenSpec(h = 100): TokenNodeSpec {
  return { x: 0, y: 0, w: 100, h, rotation: 0, visual: { kind: "image", url: "/a" }, borderColor: null, badges: [], shape: "square", perceived: false };
}

/** A view whose token lookups are faked; `transform` is read live on every reconcile. */
function makeView(opts: {
  store: DocumentStore;
  backend: MockBackend;
  transform?: () => TokenTransform | undefined;
  vfxAssets?: (id: string) => ResolvedVfxSource | null;
  vfxEnabled?: () => boolean;
  reducedMotion?: () => boolean;
}): VfxView {
  return new VfxView(
    opts.store,
    opts.backend,
    () => "scene1",
    opts.vfxAssets ?? ((id) => (id === "fx1" ? SHEET : null)),
    opts.transform ?? (() => ({ x: 10, y: 20, rotation: 0 })),
    () => tokenSpec(),
    opts.vfxEnabled ?? (() => true),
    opts.reducedMotion ?? (() => false),
  );
}

describe("VfxView emitters", () => {
  it("pushes an emitter node for an enabled emission and removes it when the emission is disabled", () => {
    const store = storeWithToken(vfxEmission());
    const backend = new MockBackend();
    const view = makeView({ store, backend });
    view.reconcile();
    const node = backend.vfx.get("emitter:tok1");
    expect(node).toBeDefined();
    expect(node!.x).toBe(10);
    expect(node!.y).toBe(20);
    expect(node!.anchor).toBe("token");
    expect(node!.token).toBe("tok1");
    expect(node!.loop).toBe(true);
    expect(view.count()).toBe(1);
    // The emission flips off: the next reconcile tears the node down.
    store.applyCommand({
      seq: 2, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "update", doc_id: "act1", changes: [{ path: "/engine/vfx/enabled", old: true, new: false }] }],
    });
    view.reconcile();
    expect(backend.vfx.has("emitter:tok1")).toBe(false);
    expect(view.count()).toBe(0);
  });

  it("pushes nothing when the source fails to resolve (fail closed)", () => {
    const store = storeWithToken(vfxEmission());
    const backend = new MockBackend();
    const view = makeView({ store, backend, vfxAssets: () => null });
    view.reconcile();
    expect(backend.vfx.size).toBe(0);
  });

  it("tracks the live tweened transform, never the doc-projected position", () => {
    const store = storeWithToken(vfxEmission());
    const backend = new MockBackend();
    let live = { x: 10, y: 20, rotation: 0 };
    const view = makeView({ store, backend, transform: () => live });
    view.reconcile();
    expect(backend.vfx.get("emitter:tok1")!.x).toBe(10);
    live = { x: 42, y: 24, rotation: 0 };
    view.reconcile();
    expect(backend.vfx.get("emitter:tok1")!.x).toBe(42);
    expect(backend.vfx.get("emitter:tok1")!.y).toBe(24);
  });

  it("re-reads the live transform every TICK, so an emitter follows a token mid-tween", () => {
    const store = storeWithToken(vfxEmission());
    const backend = new MockBackend();
    let live = { x: 10, y: 20, rotation: 0 };
    const view = makeView({ store, backend, transform: () => live });
    view.reconcile();
    expect(backend.vfx.get("emitter:tok1")!.x).toBe(10);
    // The token moves AFTER the last store commit — no reconcile runs; the per-tick
    // transform pass must carry the emitter along on the next tick.
    live = { x: 35, y: 44, rotation: 0 };
    view.tick(16);
    expect(backend.vfx.get("emitter:tok1")!.x).toBe(35);
    expect(backend.vfx.get("emitter:tok1")!.y).toBe(44);
    // A tick with an unchanged transform does not re-push (same object recorded).
    const pushed = backend.vfx.get("emitter:tok1");
    view.tick(16);
    expect(backend.vfx.get("emitter:tok1")!.x).toBe(35);
    expect(backend.vfx.get("emitter:tok1")).toBe(pushed);
  });

  it("offsets below/above anchors by half the token height and orders all anchors", () => {
    const store = storeWithToken(vfxEmission({ anchor: "below" }));
    const backend = new MockBackend();
    makeView({ store, backend }).reconcile();
    expect(backend.vfx.get("emitter:tok1")!.y).toBe(20 + 50);
    const store2 = storeWithToken(vfxEmission({ anchor: "above" }));
    const backend2 = new MockBackend();
    makeView({ store: store2, backend: backend2 }).reconcile();
    expect(backend2.vfx.get("emitter:tok1")!.y).toBe(20 - 50);
    expect(vfxAnchorZIndex("below")).toBe(0);
    expect(vfxAnchorZIndex("token")).toBe(1);
    expect(vfxAnchorZIndex("above")).toBe(2);
    expect(vfxAnchorZIndex("point")).toBe(1);
  });

  it("still renders an emitter under reduced motion, frozen at the last frame (startAtEnd, never plays through)", () => {
    const store = storeWithToken(vfxEmission({ loop: true }));
    const backend = new MockBackend();
    makeView({ store, backend, reducedMotion: () => true }).reconcile();
    const node = backend.vfx.get("emitter:tok1")!;
    expect(node.loop).toBe(false);
    expect(node.startAtEnd).toBe(true);
  });

  it("vfxEnabled === false tears down emitters AND live one-shots, and makes play() a no-op", () => {
    const store = storeWithToken(vfxEmission());
    const backend = new MockBackend();
    let enabled = true;
    const view = makeView({ store, backend, vfxEnabled: () => enabled });
    view.reconcile();
    view.play({ scene: "scene1", asset: "fx1", x: 1, y: 2, id: "one" });
    expect(view.count()).toBe(2);
    enabled = false;
    view.play({ scene: "scene1", asset: "fx1", x: 3, y: 4, id: "two" });
    expect(backend.vfx.has("oneshot:two")).toBe(false); // play() is a no-op while disabled
    view.reconcile();
    expect(backend.vfx.size).toBe(0); // every node torn down
    expect(view.count()).toBe(0);
  });
});

describe("VfxView one-shots", () => {
  it("plays a one-shot at the request point with scale/rotation defaults", () => {
    const backend = new MockBackend();
    const view = makeView({ store: new DocumentStore(), backend });
    view.play({ scene: "scene1", asset: "fx1", x: 5, y: 6, id: "one" });
    const node = backend.vfx.get("oneshot:one");
    expect(node).toBeDefined();
    expect(node!.x).toBe(5);
    expect(node!.y).toBe(6);
    expect(node!.scale).toBe(1);
    expect(node!.rotation).toBe(0);
    expect(node!.anchor).toBe("point");
    expect(node!.loop).toBe(false); // no durationMs: exactly one natural loop
  });

  it("removes a one-shot on the backend's onDone — from the bookkeeping AND the display list", () => {
    const backend = new MockBackend();
    const view = makeView({ store: new DocumentStore(), backend });
    view.play({ scene: "scene1", asset: "fx1", x: 0, y: 0, id: "one" });
    expect(view.count()).toBe(1);
    backend.completeVfxForTest("oneshot:one");
    view.tick(16);
    expect(view.count()).toBe(0);
    expect(backend.vfx.has("oneshot:one")).toBe(false); // no forever-rendered final frame
  });

  it("drops one-shots that do not belong to the viewed scene on the next reconcile", () => {
    const backend = new MockBackend();
    let viewed: string | null = "s1";
    const store = new DocumentStore();
    const view = new VfxView(
      store,
      backend,
      () => viewed,
      (id) => (id === "fx1" ? SHEET : null),
      () => undefined,
      () => undefined,
    );
    view.play({ scene: "s1", asset: "fx1", x: 0, y: 0, id: "one" });
    view.play({ scene: "s2", asset: "fx1", x: 1, y: 1, id: "two" });
    expect(view.count()).toBe(2);
    viewed = "s2";
    view.reconcile();
    expect(backend.vfx.has("oneshot:one")).toBe(false); // scene-1 coords mean nothing over s2
    expect(backend.vfx.has("oneshot:two")).toBe(true);
    expect(view.count()).toBe(1);
  });

  it("force-removes a durationMs-capped one-shot once elapsed exceeds the cap", () => {
    const backend = new MockBackend();
    const view = makeView({ store: new DocumentStore(), backend });
    view.play({ scene: "scene1", asset: "fx1", x: 0, y: 0, id: "one", durationMs: 100 });
    expect(backend.vfx.get("oneshot:one")!.loop).toBe(true); // the clip loops to fill the window
    view.tick(60);
    expect(view.count()).toBe(1);
    view.tick(60); // 120ms elapsed > 100ms cap — the underlying loop never fired onDone
    expect(backend.vfx.has("oneshot:one")).toBe(false);
    expect(view.count()).toBe(0);
  });

  it("evicts the oldest live one-shot past the 64 cap", () => {
    const backend = new MockBackend();
    const view = makeView({ store: new DocumentStore(), backend });
    for (let i = 1; i <= 65; i++) {
      view.play({ scene: "scene1", asset: "fx1", x: i, y: 0, id: `s${i}` });
    }
    expect(backend.vfx.has("oneshot:s1")).toBe(false);
    for (let i = 2; i <= 65; i++) {
      expect(backend.vfx.has(`oneshot:s${i}`)).toBe(true);
    }
    expect(view.count()).toBe(64);
  });

  it("is a complete no-op under reduced motion", () => {
    const backend = new MockBackend();
    const view = makeView({ store: new DocumentStore(), backend, reducedMotion: () => true });
    view.play({ scene: "scene1", asset: "fx1", x: 0, y: 0, id: "one" });
    expect(backend.vfx.size).toBe(0);
    expect(view.count()).toBe(0);
  });
});
