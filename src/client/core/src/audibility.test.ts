// @vitest-environment node
import { describe, it, expect } from "vitest";
import {
  EMPTY_AUDIBILITY,
  EMPTY_SCENE_AUDIBILITY,
  parseAudibility,
  sceneAudibility,
  type AudibilityPayload,
} from "./audibility";

/** A valid one-scene payload with one emitter. */
function validPayload(): unknown {
  return {
    scenes: [
      {
        scene: "scene-1",
        listener: "tok-9",
        spatial: true,
        emitters: [{ token: "tok-1", asset: "a-wind", gain: 0.5, pan: -0.25, loop: true }],
      },
      {
        scene: "scene-2",
        listener: null,
        spatial: false,
        emitters: [],
      },
    ],
  };
}

describe("parseAudibility", () => {
  it("parses a valid multi-scene payload verbatim", () => {
    const parsed = parseAudibility(validPayload());
    expect(parsed.scenes).toHaveLength(2);
    expect(parsed.scenes[0]).toEqual({
      scene: "scene-1",
      listener: "tok-9",
      spatial: true,
      emitters: [{ token: "tok-1", asset: "a-wind", gain: 0.5, pan: -0.25, loop: true }],
    });
    expect(parsed.scenes[1]).toEqual({ scene: "scene-2", listener: null, spatial: false, emitters: [] });
  });

  it("fails closed to EMPTY_AUDIBILITY on a garbled payload", () => {
    expect(parseAudibility(null)).toBe(EMPTY_AUDIBILITY);
    expect(parseAudibility({ scenes: "nope" })).toBe(EMPTY_AUDIBILITY);
    // A half-valid payload (one good scene, one garbled) fails WHOLE, never a partial read.
    const mixed = validPayload() as AudibilityPayload;
    (mixed.scenes[1] as unknown as Record<string, unknown>).spatial = "yes";
    expect(parseAudibility(mixed)).toBe(EMPTY_AUDIBILITY);
    // A non-finite gain fails the payload (a garbled emitter must never reach a live node).
    const bad = validPayload() as AudibilityPayload;
    bad.scenes[0].emitters[0].gain = Number.NaN;
    expect(parseAudibility(bad)).toBe(EMPTY_AUDIBILITY);
  });
});

describe("sceneAudibility", () => {
  it("picks the slice for the viewed scene", () => {
    const parsed = parseAudibility(validPayload());
    const slice = sceneAudibility(parsed, "scene-2");
    expect(slice.scene).toBe("scene-2");
    expect(slice.spatial).toBe(false);
  });

  it("returns the empty slice for an absent scene id and for null", () => {
    const parsed = parseAudibility(validPayload());
    expect(sceneAudibility(parsed, "scene-9")).toBe(EMPTY_SCENE_AUDIBILITY);
    expect(sceneAudibility(parsed, null)).toBe(EMPTY_SCENE_AUDIBILITY);
  });
});
