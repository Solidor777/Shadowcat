// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { SceneSelection } from "./sceneSelection.svelte";

describe("SceneSelection", () => {
  it("holds and clears the configure-target scene id", () => {
    const s = new SceneSelection();
    expect(s.configureSceneId).toBeNull();
    s.select("sc1");
    expect(s.configureSceneId).toBe("sc1");
    s.select(null);
    expect(s.configureSceneId).toBeNull();
  });
});
