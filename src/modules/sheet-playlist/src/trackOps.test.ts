// @vitest-environment node
import { describe, it, expect } from "vitest";
import { addTrack, defaultTrack, moveTrack, removeTrack, setTrack } from "./trackOps";
import type { PlaylistTrack } from "@shadowcat/core";

const t = (asset: string): PlaylistTrack => ({ asset, name: null, gain: 1, loop: false });

describe("trackOps", () => {
  it("defaultTrack is a unity-gain, non-looping track", () => {
    expect(defaultTrack("a1")).toEqual({ asset: "a1", name: null, gain: 1, loop: false });
  });

  it("addTrack appends without mutating the stored array", () => {
    const stored = [t("a")];
    const next = addTrack(stored, "b");
    expect(stored).toHaveLength(1);
    expect(next.map((x) => x.asset)).toEqual(["a", "b"]);
    expect(next[0]).not.toBe(stored[0]); // structuredClone, never the store's object
  });

  it("removeTrack drops the indexed track", () => {
    expect(removeTrack([t("a"), t("b"), t("c")], 1).map((x) => x.asset)).toEqual(["a", "c"]);
  });

  it("moveTrack reorders and clamps at both bounds", () => {
    expect(moveTrack([t("a"), t("b")], 0, 1).map((x) => x.asset)).toEqual(["b", "a"]);
    expect(moveTrack([t("a"), t("b")], 1, 1).map((x) => x.asset)).toEqual(["a", "b"]);
    expect(moveTrack([t("a"), t("b")], 0, -1).map((x) => x.asset)).toEqual(["a", "b"]);
  });

  it("setTrack replaces the indexed track", () => {
    expect(setTrack([t("a"), t("b")], 1, t("c")).map((x) => x.asset)).toEqual(["a", "c"]);
  });
});
