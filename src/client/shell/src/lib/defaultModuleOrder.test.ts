import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { coreUi } from "@shadowcat/module-core-ui";
import { sidebar } from "@shadowcat/module-sidebar";
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

// Every sidebar-contributing module in App.svelte's default set, registered in
// the exact order enterWorld() passes to WorldSession. INVARIANT: exactly one
// contribution may hold the lowest `order` — a tie at the minimum order is
// resolved by registration sequence, so an unintended second order-0 (or lower)
// contributor silently becomes the default tab instead of chat.
describe("default module set — sidebar default tab", () => {
  it("chat:sidebar (order 0) is the first sidebar contribution across the full default module set", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions } as never;
    for (const m of [sidebar, coreUi, topBar, statusBar, stage, settings, gameSettings, assets, actors, factions, conditions, sceneTools, chat]) {
      m.register(ctx);
    }
    const list = contributions.contributionsFor("shadowcat.surface:sidebar");
    expect(list[0]?.id).toBe("chat:sidebar");
  });
});
