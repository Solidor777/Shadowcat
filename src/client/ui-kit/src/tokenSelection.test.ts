import { describe, it, expect } from "vitest";
import { TokenSelection } from "./tokenSelection.svelte";

describe("TokenSelection", () => {
  it("sets, toggles, and clears the selected token ids", () => {
    const sel = new TokenSelection();
    expect(sel.has("a")).toBe(false);
    sel.set(["a", "b"]);
    expect([...sel.ids].sort()).toEqual(["a", "b"]);
    sel.toggle("b");
    expect(sel.has("b")).toBe(false);
    sel.toggle("c");
    expect(sel.has("c")).toBe(true);
    sel.clear();
    expect(sel.ids.size).toBe(0);
  });
});
