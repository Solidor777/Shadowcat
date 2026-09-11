// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { tables } from "./index";

describe("tables module", () => {
  it("contributes a launcher-closed panel at order 3", () => {
    expect(tables.manifest.id).toBe("tables");
    expect(tables.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    tables.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].order).toBe(3);
    expect(list[0].panel).toEqual({
      icon: "📋",
      labelKey: "tables.tab",
    });
    expect(list[0].panel?.defaultPlacement).toBeUndefined();
  });
});
