import { test, expect } from "vitest";
import { EmoteView } from "./index";

test("an emote rises and fades over its lifetime, then drops", () => {
  const v = new EmoteView();
  v.add(10, 20, "😀", 100);
  let glyphs = v.tick(0);
  expect(glyphs).toHaveLength(1);
  expect(glyphs[0]).toMatchObject({ x: 10, y: 20, emote: "😀" });
  expect(glyphs[0].alpha).toBeCloseTo(1);

  glyphs = v.tick(1000); // half life: risen half the rise, half faded
  expect(glyphs[0].alpha).toBeCloseTo(0.5);
  expect(glyphs[0].y).toBeCloseTo(20 - 50);

  glyphs = v.tick(2000); // total age 3000 > lifetime → dropped
  expect(glyphs).toHaveLength(0);
});

test("multiple emotes animate independently, anchored at their spawn points", () => {
  const v = new EmoteView();
  v.add(0, 0, "😀", 100);
  v.tick(1500);
  v.add(5, 5, "🔥", 100); // a fresh emote while the first is mid-fade
  const glyphs = v.tick(100);
  expect(glyphs).toHaveLength(2);
  // The first emote kept its spawn anchor (x fixed) while rising.
  expect(glyphs[0].x).toBe(0);
  expect(glyphs[0].y).toBeLessThan(0);
});
