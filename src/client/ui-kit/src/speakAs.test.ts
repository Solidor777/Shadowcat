// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, expect, it } from "vitest";
import { SpeakAs } from "./speakAs.svelte";

describe("SpeakAs", () => {
  it("holds the sticky actor id and clears with an empty string", () => {
    const s = new SpeakAs();
    expect(s.actorId).toBe("");
    s.actorId = "actor-1";
    expect(s.actorId).toBe("actor-1");
    s.actorId = "";
    expect(s.actorId).toBe("");
  });
});
