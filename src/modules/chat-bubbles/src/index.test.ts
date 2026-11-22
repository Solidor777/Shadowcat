import { describe, it, expect } from "vitest";
import { ContributionRegistry, STAGE_OVERLAY_CONTRACT, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
import { chatBubbles } from "./index";

describe("chatBubbles module", () => {
  it("declares the stage-overlay provide and the settings-section require", () => {
    expect(chatBubbles.manifest.provides).toEqual([{ contract: STAGE_OVERLAY_CONTRACT, cardinality: "multi" }]);
    expect(chatBubbles.manifest.requires).toEqual([SETTINGS_SECTION_CONTRACT]);
  });

  it("contributes exactly one stage overlay carrying its preference, and one setting", () => {
    const contributions = new ContributionRegistry();
    chatBubbles.register({ contributions, hooks: { on: () => () => {} } } as never);
    const overlays = contributions.contributionsFor(STAGE_OVERLAY_CONTRACT);
    expect(overlays).toHaveLength(1);
    expect(overlays[0].id).toBe("chat-bubbles:overlay");
    expect(contributions.contributionsFor(SETTINGS_SECTION_CONTRACT)).toHaveLength(1);
  });
});
