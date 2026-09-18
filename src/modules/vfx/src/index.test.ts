// @vitest-environment node
import { describe, expect, it } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { vfx } from "./index";

describe("@shadowcat/module-vfx registration", () => {
  it("registers exactly one PANEL_CONTRACT entry (launcher-only, no defaultPlacement)", () => {
    const registry = new ContributionRegistry();
    vfx.register({ contributions: registry } as never);
    const panels = registry.contributionsFor(PANEL_CONTRACT);
    expect(panels).toHaveLength(1);
    expect(panels[0].id).toBe("vfx:panel");
    expect(panels[0].panel?.labelKey).toBe("vfx.tab");
    expect(panels[0].panel?.defaultPlacement).toBeUndefined();
  });
});
