import { describe, it, expect, vi } from "vitest";
import { ContributionRegistry, type Contribution } from "./contributions";

const c = (over: Partial<Contribution>): Contribution => ({
  id: "x",
  contract: "s:sidebar",
  component: {},
  ...over,
});

describe("ContributionRegistry", () => {
  it("returns contributions for a contract sorted by order then insertion", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "b", order: 2 }));
    r.contribute(c({ id: "a", order: 1 }));
    r.contribute(c({ id: "c" })); // order undefined → 0
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["c", "a", "b"]);
    expect(r.contributionsFor("s:other")).toEqual([]);
  });

  it("dispose removes a single contribution and notifies subscribers", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    r.subscribe(listener);
    const dispose = r.contribute(c({ id: "a" }));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(1);
    dispose();
    expect(listener).toHaveBeenCalledTimes(2);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(0);
  });

  it("removeModule drops every contribution tagged with that module", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "a" }), { module: "m1" });
    r.contribute(c({ id: "b" }), { module: "m1" });
    r.contribute(c({ id: "k" }), { module: "m2" });
    r.removeModule("m1");
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["k"]);
  });

  it("subscribe returns an unsubscribe that stops notifications", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    const off = r.subscribe(listener);
    off();
    r.contribute(c({ id: "a" }));
    expect(listener).not.toHaveBeenCalled();
  });
});
