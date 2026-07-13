import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { statusBar } from "./index";
import StatusBar from "./StatusBar.svelte";

describe("statusbar module", () => {
  it("requires the statusbar region and provides the singleton panel-dock surface", () => {
    expect(statusBar.manifest.id).toBe("statusbar");
    expect(statusBar.manifest.requires).toContain("shadowcat.surface:statusbar");
    expect(statusBar.manifest.provides).toEqual([
      { contract: "shadowcat.surface:panel-dock", cardinality: "singleton" },
    ]);
  });

  it("contributes StatusBar into the statusbar surface", () => {
    const contributions = new ContributionRegistry();
    statusBar.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:statusbar");
    expect(list.length).toBe(1);
    expect(list[0].component).toBe(StatusBar);
  });
});
