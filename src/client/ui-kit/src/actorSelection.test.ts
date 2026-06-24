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

  it("exposes a keepAfterPlace preference, default off", () => {
    const sel = new ActorSelection();
    expect(sel.keepAfterPlace).toBe(false);
    sel.setKeepAfterPlace(true);
    expect(sel.keepAfterPlace).toBe(true);
  });
});
