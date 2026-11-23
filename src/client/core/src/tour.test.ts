import { describe, expect, test } from "vitest";
import { ContributionRegistry } from "./contributions";
import { TOUR_CONTRACT, isTourAnchorId, validateTourMeta } from "./tour";
import type { TourMeta } from "./tour";

/** A valid tour; each test mutates one field to prove that field is checked. */
function tour(over: Partial<TourMeta> = {}): TourMeta {
  return {
    id: "example.intro",
    labelKey: "example.tour.name",
    version: 1,
    audience: "any",
    scope: "global",
    trigger: "first-use",
    steps: [{ target: "example.panel", titleKey: "t", bodyKey: "b", placement: "bottom" }],
    ...over,
  };
}

describe("validateTourMeta", () => {
  test("accepts a valid tour and returns it", () => {
    expect(validateTourMeta(tour())).toEqual(tour());
  });

  test.each([
    ["empty id", { id: "" }],
    ["empty labelKey", { labelKey: "" }],
    ["zero version", { version: 0 }],
    ["fractional version", { version: 1.5 }],
    ["unknown audience", { audience: "everyone" }],
    ["unknown scope", { scope: "scene" }],
    ["unknown trigger", { trigger: "auto" }],
    ["no steps", { steps: [] }],
    ["selector as target", { steps: [{ target: ".rail > button", titleKey: "t", bodyKey: "b", placement: "top" }] }],
    ["unknown placement", { steps: [{ target: "a", titleKey: "t", bodyKey: "b", placement: "above" }] }],
    ["empty titleKey", { steps: [{ target: "a", titleKey: "", bodyKey: "b", placement: "top" }] }],
    ["unknown key", { extra: true }],
  ])("rejects %s", (_name, over) => {
    expect(() => validateTourMeta({ ...tour(), ...over })).toThrow(/invalid tour/);
  });

  test("names the failing path", () => {
    expect(() => validateTourMeta(tour({ version: 0 }))).toThrow(/version/);
  });
});

describe("isTourAnchorId", () => {
  test("accepts namespaced ids and rejects selector characters", () => {
    expect(isTourAnchorId("scene-tools.rail")).toBe(true);
    expect(isTourAnchorId("a:b_c")).toBe(true);
    expect(isTourAnchorId("")).toBe(false);
    expect(isTourAnchorId('a"]')).toBe(false);
    expect(isTourAnchorId("a b")).toBe(false);
  });
});

describe("ContributionRegistry tour validation", () => {
  test("registers a valid tour contribution", () => {
    const registry = new ContributionRegistry();
    registry.contribute({ id: "tour-a", contract: TOUR_CONTRACT, component: null, tour: tour() });
    expect(registry.contributionsFor(TOUR_CONTRACT)[0].tour?.id).toBe("example.intro");
  });

  test("rejects a tour contribution with no tour", () => {
    const registry = new ContributionRegistry();
    expect(() => registry.contribute({ id: "tour-a", contract: TOUR_CONTRACT, component: null })).toThrow(/invalid tour/);
    expect(registry.contributionsFor(TOUR_CONTRACT)).toHaveLength(0);
  });

  test("rejects an invalid tour and leaves the registry unchanged", () => {
    const registry = new ContributionRegistry();
    expect(() =>
      registry.contribute({ id: "tour-a", contract: TOUR_CONTRACT, component: null, tour: tour({ steps: [] }) }),
    ).toThrow(/invalid tour/);
    expect(registry.contributionsFor(TOUR_CONTRACT)).toHaveLength(0);
  });

  test("rejects an illegal tourAnchor on any contract", () => {
    const registry = new ContributionRegistry();
    expect(() =>
      registry.contribute({ id: "x", contract: "s:bar", component: null, tourAnchor: 'a"] b' }),
    ).toThrow(/invalid tour anchor/);
  });
});
