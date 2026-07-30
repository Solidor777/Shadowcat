import { describe, it, expect } from "vitest";
import { rollInitiative, sortEntries, type Entry } from "./index";

describe("rollInitiative", () => {
  it("stays within 1..=20", () => {
    for (let i = 0; i < 200; i++) {
      const r = rollInitiative(() => Math.random());
      expect(r).toBeGreaterThanOrEqual(1);
      expect(r).toBeLessThanOrEqual(20);
    }
  });
  it("is deterministic given a fixed rng", () => {
    expect(rollInitiative(() => 0)).toBe(1);
    expect(rollInitiative(() => 0.999999)).toBe(20);
  });
});

describe("sortEntries", () => {
  it("orders by initiative descending, name ascending on ties", () => {
    const entries: Entry[] = [
      { actorId: "usr_test_001", name: "MOCK_ACTOR_B", initiative: 12 },
      { actorId: "usr_test_002", name: "MOCK_ACTOR_A", initiative: 18 },
      { actorId: "usr_test_003", name: "MOCK_ACTOR_C", initiative: 12 },
    ];
    expect(sortEntries(entries).map((e) => e.name)).toEqual([
      "MOCK_ACTOR_A", "MOCK_ACTOR_B", "MOCK_ACTOR_C",
    ]);
  });
});
