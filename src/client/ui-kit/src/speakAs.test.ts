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
