import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { sidebar } from "./index";
import SidebarHost from "./SidebarHost.svelte";

describe("sidebar module", () => {
  it("provides the multi sidebar contract and requires sidebar-host", () => {
    expect(sidebar.manifest.id).toBe("sidebar");
    expect(sidebar.manifest.requires).toContain("shadowcat.surface:sidebar-host");
    expect(sidebar.manifest.provides).toContainEqual({
      contract: "shadowcat.surface:sidebar",
      cardinality: "multi",
    });
  });

  it("contributes SidebarHost into the sidebar-host surface", () => {
    const contributions = new ContributionRegistry();
    sidebar.register({ contributions } as never);
    const contributed = contributions.contributionsFor("shadowcat.surface:sidebar-host");
    expect(contributed.length).toBe(1);
    expect(contributed[0].component).toBe(SidebarHost);
  });
});
