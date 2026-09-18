// @vitest-environment node
import { describe, it, expect, vi } from "vitest";
import { playThrowSound } from "./audioSeam";

describe("playThrowSound", () => {
  it("plays the sound through the sfx channel when given an asset id", () => {
    const playOneShot = vi.fn();
    playThrowSound({ audio: { playOneShot } } as unknown as Parameters<typeof playThrowSound>[0], "snd-1");
    expect(playOneShot).toHaveBeenCalledWith("snd-1", { channel: "sfx" });
  });

  it("never calls playOneShot for a null sound", () => {
    const playOneShot = vi.fn();
    playThrowSound({ audio: { playOneShot } } as unknown as Parameters<typeof playThrowSound>[0], null);
    expect(playOneShot).not.toHaveBeenCalled();
  });
});
