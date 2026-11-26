// @vitest-environment node
// Pure function; no DOM.
import { test, expect } from "vitest";
import type { WallEngine } from "@shadowcat/core";
import { wallBlocksSound } from "./wall-flags";

/** A wall engine body with the given flag overrides. */
function wall(over: Partial<WallEngine>): WallEngine {
  return {
    seg: { x1: 0, y1: 0, x2: 10, y2: 0 },
    blocksSight: true, blocksLight: true, blocksMove: true, blocksSound: null,
    elevation: null, door: null, direction: null, sightTier: null, proximity: null,
    ...over,
  };
}

test("an unauthored blocksSound inherits blocksSight", () => {
  expect(wallBlocksSound(wall({ blocksSound: null }))).toBe(true);
  expect(wallBlocksSound(wall({ blocksSound: undefined }))).toBe(true);
  expect(wallBlocksSound(wall({ blocksSight: false, blocksSound: null }))).toBe(false);
  expect(wallBlocksSound(wall({ blocksSight: null, blocksSound: null }))).toBe(false);
});

test("an explicit blocksSound wins over blocksSight", () => {
  expect(wallBlocksSound(wall({ blocksSound: false }))).toBe(false);
  expect(wallBlocksSound(wall({ blocksSight: false, blocksSound: true }))).toBe(true);
});
