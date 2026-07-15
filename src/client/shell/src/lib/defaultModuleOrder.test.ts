import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { coreUi } from "@shadowcat/module-core-ui";
import { panels } from "@shadowcat/module-panels";
import { topBar } from "@shadowcat/module-topbar";
import { statusBar } from "@shadowcat/module-statusbar";
import { stage } from "@shadowcat/module-stage";
import { settings } from "@shadowcat/module-settings";
import { assets } from "@shadowcat/module-assets";
import { actors } from "@shadowcat/module-actors";
import { factions } from "@shadowcat/module-factions";
import { conditions } from "@shadowcat/module-conditions";
import { gameSettings } from "@shadowcat/module-game-settings";
import { sceneTools } from "@shadowcat/module-scene-tools";
import { chat } from "@shadowcat/module-chat";
import { defaultLayout } from "@shadowcat/module-panels";

// Every panel-contributing module in App.svelte's default set, registered in the
// exact order enterWorld() passes to WorldSession. INVARIANT: exactly one
// contribution may hold the lowest `order` — a tie at the minimum order is
// resolved by registration sequence, so an unintended second order-0 (or lower)
// contributor silently becomes the default docked panel instead of chat.
describe("default module set — default docked panel", () => {
  it("chat:panel (order 0) is the first shadowcat.panel contribution across the full default module set", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions } as never;
    for (const m of [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, assets, actors, factions, conditions, sceneTools, chat]) {
      m.register(ctx);
    }
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list[0]?.id).toBe("chat:panel");
  });

  it("the built default layout docks exactly chat:panel; every other panel starts minimized", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions } as never;
    for (const m of [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, assets, actors, factions, conditions, sceneTools, chat]) {
      m.register(ctx);
    }
    const regs = contributions.contributionsFor(PANEL_CONTRACT).map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement }));
    const layout = defaultLayout(regs);

    const docked = Object.values(layout.expanded.zones).flatMap((z) => z.groups.flatMap((g) => g.tabs));
    expect(docked).toEqual(["chat:panel"]);
    expect(layout.expanded.minimized.sort()).toEqual(
      ["assets:panel", "actors:panel", "factions:panel", "conditions:panel", "game-settings:panel", "settings:panel"].sort(),
    );
  });
});
