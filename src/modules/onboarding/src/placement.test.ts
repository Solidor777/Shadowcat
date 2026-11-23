import { describe, expect, it } from "vitest";
import { CARD_GAP, VIEWPORT_MARGIN, placeCard } from "./placement";

const card = { width: 200, height: 80 };
const viewport = { width: 1000, height: 800 };
const target = { left: 400, top: 300, width: 100, height: 40 };

describe("placeCard", () => {
  it("puts the card below the target, centred on it", () => {
    expect(placeCard(target, "bottom", card, viewport)).toEqual({ left: 350, top: 300 + 40 + CARD_GAP });
  });

  it("puts the card above, left of and right of the target", () => {
    expect(placeCard(target, "top", card, viewport)).toEqual({ left: 350, top: 300 - CARD_GAP - 80 });
    expect(placeCard(target, "left", card, viewport)).toEqual({ left: 400 - CARD_GAP - 200, top: 320 - 40 });
    expect(placeCard(target, "right", card, viewport)).toEqual({ left: 500 + CARD_GAP, top: 320 - 40 });
  });

  it("centres the card for the center placement and for a missing target", () => {
    expect(placeCard(target, "center", card, viewport)).toEqual({ left: 400, top: 360 });
    expect(placeCard(null, "bottom", card, viewport)).toEqual({ left: 400, top: 360 });
  });

  it("shifts the card back inside the viewport", () => {
    const corner = { left: 990, top: 790, width: 10, height: 10 };
    expect(placeCard(corner, "bottom", card, viewport)).toEqual({
      left: 1000 - 200 - VIEWPORT_MARGIN,
      top: 800 - 80 - VIEWPORT_MARGIN,
    });
    expect(placeCard({ left: 0, top: 0, width: 10, height: 10 }, "top", card, viewport)).toEqual({
      left: VIEWPORT_MARGIN,
      top: VIEWPORT_MARGIN,
    });
  });
});
