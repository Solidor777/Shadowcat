// @vitest-environment node
// Exercises plain document-store reads (no component render, no DOM API use).
import { test, expect } from "vitest";
import { DocumentStore, buildRegionDoc, buildSceneDoc, buildTokenDoc, type RegionEngine, type RegionTrigger } from "@shadowcat/core";
import { offersPortal, portalTokens } from "./portal-use";

const choice: RegionTrigger = {
  on: "enter",
  effect: { type: "portal_choice", prompt: null, options: [{ label: "A", target: { scene: null, x: 0, y: 0, elevation: null, vfx: null }, reveal_scene: false }] },
};

/** A 100x100 rect region at the origin carrying `triggers`. */
function region(triggers: RegionTrigger[], over: Partial<RegionEngine> = {}, id = "r1", scene = "s1") {
  return buildRegionDoc("w1", scene, {
    shape: { kind: "rect", points: [0, 0, 100, 100] },
    behavior: "terrain", cost: 1, cost_by_tag: {}, enabled: true, triggers, elevation: null, ambient_override: null, occludes: null, anchor: null,
    ...over,
  }, id);
}

/** A token centred at (x, y) in scene `scene`. */
function token(id: string, x: number, y: number, scene = "s1") {
  return buildTokenDoc("w1", scene, {
    x, y, w: 10, h: 10, rotation: 0,
    visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false,
  }, id);
}

/** A store holding both scenes and `docs`. */
function store(...docs: ReturnType<typeof region>[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [buildSceneDoc("w1", {}, "s1"), buildSceneDoc("w1", {}, "s2"), ...docs].map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

test("offersPortal needs an enabled region with a portal_choice trigger on a non-exit event", () => {
  expect(offersPortal(region([choice]))).toBe(true);
  expect(offersPortal(region([{ ...choice, on: "arrest" }]))).toBe(true);
  expect(offersPortal(region([{ ...choice, on: "exit" }]))).toBe(false);
  expect(offersPortal(region([choice], { enabled: false }))).toBe(false);
  expect(offersPortal(region([]))).toBe(false);
  expect(offersPortal(region([{ on: "enter", effect: { type: "teleport", target: choice.effect.type === "portal_choice" ? choice.effect.options[0].target : (null as never) } }]))).toBe(false);
});

test("offersPortal reads a region document with no triggers key as offering nothing", () => {
  const doc = region([choice]);
  delete (doc.engine as Partial<RegionEngine>).triggers;
  expect(offersPortal(doc)).toBe(false);
});

test("portalTokens keeps only tokens standing inside a portal region of their own scene", () => {
  const s = store(region([choice]), token("in", 50, 50), token("out", 500, 500), token("other-scene", 50, 50, "s2"));
  expect(portalTokens(["in", "out", "other-scene", "missing"], s)).toEqual(["in"]);
});

test("portalTokens returns nothing when no region offers a portal", () => {
  const s = store(region([{ ...choice, on: "exit" }]), token("in", 50, 50));
  expect(portalTokens(["in"], s)).toEqual([]);
});
