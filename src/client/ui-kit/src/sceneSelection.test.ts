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
