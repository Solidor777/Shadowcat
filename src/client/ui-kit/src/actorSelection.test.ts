import { describe, it, expect } from "vitest";
import { ActorSelection } from "./actorSelection.svelte";

describe("ActorSelection", () => {
  it("holds and updates the selected actor id (stable instance)", () => {
    const sel = new ActorSelection();
    expect(sel.selectedId).toBeNull();
    sel.select("act1");
    expect(sel.selectedId).toBe("act1");
    sel.select(null);
    expect(sel.selectedId).toBeNull();
  });
});
