import { describe, it, expect } from "vitest";
import { flushSync } from "svelte";
import { Camera } from "@shadowcat/render";
import { StageView } from "./stageView.svelte";

describe("StageView", () => {
  it("is the identity before the first transform", () => {
    expect(new StageView().sceneToScreen({ x: 12, y: -4 })).toEqual({ x: 12, y: -4 });
  });

  it("projects with scale and offset, matching Camera.sceneToScreen", () => {
    const camera = new Camera();
    camera.panBy(30, -20);
    camera.zoomAt(2, 100, 50);
    const view = new StageView();
    view.set(camera.transform());
    for (const p of [{ x: 0, y: 0 }, { x: 250, y: 90 }, { x: -40, y: 7 }]) {
      expect(view.sceneToScreen(p)).toEqual(camera.sceneToScreen(p));
    }
  });

  it("reveals nothing until a probe is attached, then follows it, and fails closed again on detach", () => {
    const view = new StageView();
    expect(view.isTokenRevealed("t", { x: 0, y: 0 })).toBe(false);
    const detach = view.attachReveal((id) => id === "t");
    expect(view.isTokenRevealed("t", { x: 0, y: 0 })).toBe(true);
    expect(view.isTokenRevealed("u", { x: 0, y: 0 })).toBe(false);
    detach();
    expect(view.isTokenRevealed("t", { x: 0, y: 0 })).toBe(false);
  });

  it("a stale detach does not clear a newer probe", () => {
    const view = new StageView();
    const staleDetach = view.attachReveal(() => false);
    view.attachReveal(() => true);
    staleDetach();
    expect(view.isTokenRevealed("t", { x: 0, y: 0 })).toBe(true);
  });

  it("a mask announcement re-runs a reactive reader", () => {
    const view = new StageView();
    let open = false;
    view.attachReveal(() => open);
    const seen: boolean[] = [];
    const stop = $effect.root(() => {
      $effect(() => { seen.push(view.isTokenRevealed("t", { x: 0, y: 0 })); });
    });
    flushSync();
    open = true;
    view.noteMaskApplied();
    flushSync();
    stop();
    expect(seen).toEqual([false, true]);
  });

  it("set replaces the transform in place and is not aliased to the caller's object", () => {
    const view = new StageView();
    const t = { x: 5, y: 6, scale: 3 };
    view.set(t);
    t.x = 999;
    expect(view.transform).toEqual({ x: 5, y: 6, scale: 3 });
  });
});
