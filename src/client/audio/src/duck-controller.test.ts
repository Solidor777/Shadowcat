// @vitest-environment node
import { describe, it, expect } from "vitest";
import { approach, DuckControllerImpl, DEFAULT_DUCK_DEPTH } from "./duck-controller";

describe("approach", () => {
  it("moves one time constant of the way by definition", () => {
    expect(approach(0, 1, 50, 50)).toBeCloseTo(0.632, 2);
    expect(approach(1, 0, 50, 50)).toBeCloseTo(0.368, 2);
  });

  it("a non-positive tau jumps straight to the target", () => {
    expect(approach(0, 1, 50, 0)).toBe(1);
  });
});

describe("DuckControllerImpl", () => {
  it("reports gain 1 at rest", () => {
    expect(new DuckControllerImpl().gain).toBe(1);
  });

  it("reports the constructor's depth and a clamped setDepth", () => {
    expect(new DuckControllerImpl(0.2).depth).toBe(0.2);
    const duck = new DuckControllerImpl();
    duck.setDepth(0.4);
    expect(duck.depth).toBe(0.4);
    duck.setDepth(1.5);
    expect(duck.depth).toBe(1);
    duck.setDepth(-1);
    expect(duck.depth).toBe(0);
  });

  it("takes the MAX demand across sources, never a sum", () => {
    const duck = new DuckControllerImpl();
    const a = duck.addSource("a");
    const b = duck.addSource("b");
    a.set(0.5);
    b.set(0.9);
    duck.tick(1_000);
    duck.tick(1_100);
    const target = 1 - 0.9 * DEFAULT_DUCK_DEPTH;
    // Two full-ish demands duck no harder than the max one alone.
    expect(duck.gain).toBeGreaterThan(target);
    duck.removeSource("b");
    duck.tick(1_200);
    duck.removeSource("a");
  });

  it("attack is faster than release from the same starting point", () => {
    const duck = new DuckControllerImpl();
    const src = duck.addSource("x");
    src.set(1);
    duck.tick(0);
    duck.tick(100);
    const dipped = duck.gain; // attack (50ms tau) has driven most of the way down
    src.set(0);
    duck.tick(200);
    // Release (600ms tau) recovers far less in the same 100ms than attack dropped.
    const dropped = 1 - dipped;
    const recovered = duck.gain - dipped;
    expect(dropped).toBeGreaterThan(0);
    expect(recovered).toBeGreaterThan(0);
    expect(recovered).toBeLessThan(dropped);
  });

  it("setDepth re-targets the NEXT tick, not the current gain instantly", () => {
    const duck = new DuckControllerImpl(0.2);
    const src = duck.addSource("x");
    src.set(1);
    duck.tick(0);
    duck.tick(100);
    const before = duck.gain;
    duck.setDepth(0.9);
    duck.tick(200);
    // The smoothed demand (~0.865 after 100ms at the attack tau) now applies the
    // deeper depth — gain moved but did not jump to the 1 - 0.9 floor.
    expect(duck.gain).toBeLessThan(before);
    expect(duck.gain).toBeGreaterThan(1 - 0.9);
  });
});
