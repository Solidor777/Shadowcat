// @vitest-environment node
// Exercises plain controller state (no component render, no DOM API use).
import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, type RegionTrigger } from "@shadowcat/core";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { ToolController, type ToolContext } from "./controller.svelte";

function setup(): { ctx: ToolContext; controller: ToolController; gmViewedScenes: (string | null)[] } {
  const docs = new DocumentStore();
  docs.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: buildSceneDoc("w1", {}, "s1") }] });
  const bridge = new SceneInteractionBridge();
  const activeTools: unknown[] = [];
  bridge.attach(fakeSceneHost({ setActiveTool: (t) => activeTools.push(t) }));
  const gmViewedScenes: (string | null)[] = [];
  const ctx: ToolContext = {
    scene: bridge,
    dispatchIntent: () => {},
    documents: docs,
    assets: new AssetResolver(),
    world: "w1",
    role: "gm",
    sendPing: () => {},
    t: (k) => k,
    viewedSceneId: () => "s1",
    setGmViewedScene: (id) => gmViewedScenes.push(id),
  };
  const controller = new ToolController(ctx);
  return { ctx, controller, gmViewedScenes };
}

function teleportTrigger(scene: string | null): RegionTrigger {
  return { on: "enter", effect: { type: "teleport", target: { scene, x: 0, y: 0, elevation: null, vfx: null } } };
}

test("beginPickPortalTarget cleanly cancels an in-progress pick before starting a new one, never stranding the GM on an intermediate scene", () => {
  const { controller, gmViewedScenes } = setup();
  controller.regionTriggers = [teleportTrigger("row0-dest"), teleportTrigger("row1-dest")];

  controller.beginPickPortalTarget(0);
  expect(gmViewedScenes).toEqual(["row0-dest"]); // roamed to row 0's destination
  expect(controller.pickingPortalRow).toBe(0);

  // Start a SECOND pick while the first is still in progress. Without the re-entrancy guard,
  // `#pickOriginalScene` would be overwritten with "row0-dest" (the first pick's already-roamed-to
  // destination) instead of the TRUE original "s1".
  controller.beginPickPortalTarget(1);
  expect(controller.pickingPortalRow).toBe(1);
  // Cancelling row 0's pick restored "s1" before row 1's pick roamed to its own destination.
  expect(gmViewedScenes).toEqual(["row0-dest", "s1", "row1-dest"]);

  // Ending row 1's pick restores the TRUE original scene, not row 0's destination.
  controller.endPickPortalTarget(null);
  expect(gmViewedScenes).toEqual(["row0-dest", "s1", "row1-dest", "s1"]);
});

test("endPickPortalTarget resolves the target by the captured trigger OBJECT, not the row index captured at pick-start", () => {
  const { controller } = setup();
  const rowToRemove = teleportTrigger(null);
  const rowBeingPicked = teleportTrigger(null);
  controller.regionTriggers = [rowToRemove, rowBeingPicked];

  controller.beginPickPortalTarget(1); // picking for `rowBeingPicked`, currently at index 1

  // Remove the row ahead of it: `rowBeingPicked` now sits at index 0, but the pick must still
  // target IT, never whatever now occupies index 1.
  controller.regionTriggers.splice(0, 1);
  expect(controller.regionTriggers).toEqual([rowBeingPicked]);

  controller.endPickPortalTarget({ x: 12, y: 34 });
  expect(rowBeingPicked.effect).toEqual({ type: "teleport", target: { scene: null, x: 12, y: 34, elevation: null, vfx: null } });
});

test("endPickPortalTarget no-ops the write (but still restores the scene) when the captured trigger was removed entirely", () => {
  const { controller, gmViewedScenes } = setup();
  const removed = teleportTrigger("dest");
  controller.regionTriggers = [removed];

  controller.beginPickPortalTarget(0);
  expect(gmViewedScenes).toEqual(["dest"]);
  controller.regionTriggers = []; // the row is deleted entirely mid-pick

  controller.endPickPortalTarget({ x: 1, y: 2 });
  // No write happened — the removed trigger's target is untouched.
  expect(removed.effect).toEqual({ type: "teleport", target: { scene: "dest", x: 0, y: 0, elevation: null, vfx: null } });
  // The scene restore still runs regardless.
  expect(gmViewedScenes).toEqual(["dest", "s1"]);
});
