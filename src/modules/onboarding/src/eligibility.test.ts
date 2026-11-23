import { describe, expect, it } from "vitest";
import { matchesAudience, offerableTours, resolvableSteps, type TourViewer } from "./eligibility";
import { GM, memorySettings, mountedAnchors, threeStepTour, tour } from "./__fixtures__/harness";
import { TourProgress } from "./progress";

const PLAYER: TourViewer = { world: "w1", role: "player", serverRole: "user" };
const SPECTATOR: TourViewer = { world: "w1", role: "spectator", serverRole: "user" };
const ADMIN: TourViewer = { world: "w1", role: "player", serverRole: "admin" };

describe("matchesAudience", () => {
  it("matches each audience to the right viewer", () => {
    expect(matchesAudience("any", SPECTATOR)).toBe(true);
    expect(matchesAudience("gm", GM)).toBe(true);
    expect(matchesAudience("gm", PLAYER)).toBe(false);
    expect(matchesAudience("player", PLAYER)).toBe(true);
    expect(matchesAudience("player", GM)).toBe(false);
    expect(matchesAudience("player", SPECTATOR)).toBe(false);
    expect(matchesAudience("admin", ADMIN)).toBe(true);
    expect(matchesAudience("admin", GM)).toBe(false);
  });
});

describe("resolvableSteps", () => {
  it("skips a step whose anchor is not mounted", () => {
    expect(resolvableSteps(threeStepTour(), mountedAnchors(["example.a", "example.c"]))).toEqual([0, 2]);
  });

  it("returns nothing when no anchor is mounted", () => {
    expect(resolvableSteps(threeStepTour(), mountedAnchors([]))).toEqual([]);
  });
});

describe("offerableTours", () => {
  const resolve = mountedAnchors(["example.a"]);
  const fresh = () => new TourProgress(memorySettings(), () => {});

  it("offers a first-use tour for the viewer's audience", () => {
    expect(offerableTours([tour()], GM, fresh(), resolve).map((t) => t.id)).toEqual(["example.intro"]);
  });

  it("does not offer a tour meant for another role", () => {
    expect(offerableTours([tour({ audience: "gm" })], PLAYER, fresh(), resolve)).toEqual([]);
  });

  it("does not offer a manual tour", () => {
    expect(offerableTours([tour({ trigger: "manual" })], GM, fresh(), resolve)).toEqual([]);
  });

  it("does not offer a tour whose version the user already finished", () => {
    const progress = new TourProgress(memorySettings({ "g:example.intro": 1 }), () => {});
    expect(offerableTours([tour({ version: 1 })], GM, progress, resolve)).toEqual([]);
  });

  it("offers the tour again after its version is bumped", () => {
    const progress = new TourProgress(memorySettings({ "g:example.intro": 1 }), () => {});
    expect(offerableTours([tour({ version: 2 })], GM, progress, resolve)).toHaveLength(1);
  });

  it("offers a world-scoped tour in a world where it was not finished", () => {
    const progress = new TourProgress(memorySettings({ "w:w1:example.intro": 1 }), () => {});
    const scoped = tour({ scope: "world" });
    expect(offerableTours([scoped], GM, progress, resolve)).toEqual([]);
    expect(offerableTours([scoped], { ...GM, world: "w2" }, progress, resolve)).toHaveLength(1);
  });

  it("offers nothing once the user switched tours off", () => {
    const progress = new TourProgress(memorySettings({ disabled: true }), () => {});
    expect(offerableTours([tour()], GM, progress, resolve)).toEqual([]);
  });

  it("does not offer a tour whose every target is missing", () => {
    expect(offerableTours([tour()], GM, fresh(), mountedAnchors([]))).toEqual([]);
  });

  it("keeps registration order", () => {
    const tours = [tour({ id: "example.first" }), tour({ id: "example.second" })];
    expect(offerableTours(tours, GM, fresh(), resolve).map((t) => t.id)).toEqual(["example.first", "example.second"]);
  });
});
