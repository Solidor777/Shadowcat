import { test, expect } from "vitest";
import { EmoteView, MAX_GLYPH_DISPLAY_CHARS, MAX_OVERLAY_DISPLAY_CHARS, truncateForDisplay } from "./index";

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

test("add defaults kind to \"glyph\" when omitted", () => {
  const v = new EmoteView();
  v.add(0, 0, "😀", 100);
  expect(v.tick(0)[0].kind).toBe("glyph");
});

test("add tracks an explicit \"caption\" kind through tick", () => {
  const v = new EmoteView();
  v.add(0, 0, "You feel a chill.", 100, "caption");
  expect(v.tick(0)[0].kind).toBe("caption");
});

test("add display-truncates text over MAX_OVERLAY_DISPLAY_CHARS with a trailing ellipsis, \
regardless of kind — mutation: raising this cap (or skipping the call) would let an unbounded \
tick(0)[0].emote length back through", () => {
  const v = new EmoteView();
  const long = "x".repeat(MAX_OVERLAY_DISPLAY_CHARS + 50);
  v.add(0, 0, long, 100, "caption");
  const emote = v.tick(0)[0].emote;
  expect(emote.endsWith("…")).toBe(true);
  expect(Array.from(emote).length).toBe(MAX_OVERLAY_DISPLAY_CHARS + 1); // + the ellipsis char
});

test("add never truncates text at or under the cap", () => {
  const v = new EmoteView();
  const short = "x".repeat(MAX_OVERLAY_DISPLAY_CHARS);
  v.add(0, 0, short, 100, "caption");
  expect(v.tick(0)[0].emote).toBe(short);
});

test("a \"glyph\" overlay display-truncates at its own far tighter cap", () => {
  // The glyph treatment draws one unwrapped line at glyph size, so its cap is much tighter than
  // the wrapped caption's. Mutation: capping every kind at MAX_OVERLAY_DISPLAY_CHARS leaves this
  // string untruncated and fails both assertions.
  const long = "x".repeat(MAX_GLYPH_DISPLAY_CHARS + 20);
  expect(MAX_GLYPH_DISPLAY_CHARS).toBeLessThan(MAX_OVERLAY_DISPLAY_CHARS);
  const out = truncateForDisplay(long, "glyph");
  expect(out.endsWith("…")).toBe(true);
  expect(Array.from(out).length).toBe(MAX_GLYPH_DISPLAY_CHARS + 1);
});

test("EmoteView.add applies the glyph cap to a default-kind overlay", () => {
  // A module calling the public addEmote seam passes no kind; the display cap is what bounds the
  // node, since no wire field does. Mutation: passing the caption cap here fails the length check.
  const v = new EmoteView();
  v.add(0, 0, "y".repeat(MAX_OVERLAY_DISPLAY_CHARS), 100);
  expect(Array.from(v.tick(0)[0].emote).length).toBe(MAX_GLYPH_DISPLAY_CHARS + 1);
});

test("truncateForDisplay is codepoint-safe: it never splits a surrogate pair at the cut point", () => {
  // An emoji (surrogate pair) placed exactly straddling the cap.
  const prefix = "x".repeat(MAX_OVERLAY_DISPLAY_CHARS - 1);
  const text = prefix + "😀" + "y".repeat(10);
  // "caption" explicitly: the default kind caps at MAX_GLYPH_DISPLAY_CHARS, which would put the
  // cut inside the prefix and never exercise the surrogate boundary this test exists for.
  const out = truncateForDisplay(text, "caption");
  // A raw UTF-16 slice at MAX_OVERLAY_DISPLAY_CHARS would cut the emoji's low surrogate off,
  // producing an unpaired surrogate (a replacement-character-prone malformed string).
  expect(out.endsWith("…")).toBe(true);
  expect([...out].every((ch) => !/[\uD800-\uDFFF]/.test(ch) || ch.length === 2)).toBe(true);
});
