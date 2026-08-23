import { describe, it, expect } from "vitest";
import { SpeakAsToken } from "./speakAsToken.svelte";

describe("SpeakAsToken", () => {
  it("holds and clears the pending token id via select", () => {
    const s = new SpeakAsToken();
    expect(s.tokenId).toBeNull();
    s.select("tok-1");
    expect(s.tokenId).toBe("tok-1");
    s.select(null);
    expect(s.tokenId).toBeNull();
  });

  it("consume reads and clears in one step", () => {
    const s = new SpeakAsToken();
    s.select("tok-1");
    expect(s.consume()).toBe("tok-1");
    expect(s.tokenId).toBeNull();
  });

  it("consume returns null when nothing is pending", () => {
    const s = new SpeakAsToken();
    expect(s.consume()).toBeNull();
  });
});
