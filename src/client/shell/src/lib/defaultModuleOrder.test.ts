import { describe, it, expect, afterEach } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT, STAGE_OVERLAY_CONTRACT } from "@shadowcat/core";
import { coreUi } from "@shadowcat/module-core-ui";
import { panels } from "@shadowcat/module-panels";
import { topBar } from "@shadowcat/module-topbar";
import { statusBar } from "@shadowcat/module-statusbar";
import { stage } from "@shadowcat/module-stage";
import { settings } from "@shadowcat/module-settings";
import { assetBrowser } from "@shadowcat/module-asset-browser";
import { actors } from "@shadowcat/module-actors";
import { factions } from "@shadowcat/module-factions";
import { conditions } from "@shadowcat/module-conditions";
import { combatTracker } from "@shadowcat/module-combat-tracker";
import { gameSettings } from "@shadowcat/module-game-settings";
import { sceneTools } from "@shadowcat/module-scene-tools";
import { chat } from "@shadowcat/module-chat";
import { notes } from "@shadowcat/module-notes";
import { tables } from "@shadowcat/module-tables";
import { defaultLayout } from "@shadowcat/module-panels";
import { sheetFallback } from "@shadowcat/module-sheet-fallback";
import { sheetActor } from "@shadowcat/module-sheet-actor";
import { sheetItem } from "@shadowcat/module-sheet-item";
import { sheetNote } from "@shadowcat/module-sheet-note";
import { sheetTable } from "@shadowcat/module-sheet-table";
import { dice3d } from "@shadowcat/module-dice-3d";
import { audio } from "@shadowcat/module-audio";
import { sheetPlaylist } from "@shadowcat/module-sheet-playlist";
import { vfx } from "@shadowcat/module-vfx";
import { SHEET_FALLBACK_CONTRACT, sheetContract } from "@shadowcat/core";
import { ducking } from "@shadowcat/module-ducking";
import { SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";

// Every panel-contributing module in `App`'s default set, registered in the
// exact order enterWorld() passes to WorldSession. INVARIANT: exactly one
// contribution may hold the lowest `order` — a tie at the minimum order is
// resolved by registration sequence, so an unintended second order-0 (or lower)
// contributor silently becomes the default docked panel instead of chat.
describe("default module set — default docked panel", () => {
  it("chat:panel (order 0) is the first shadowcat.panel contribution across the full default module set", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions, hooks: { on: () => () => {} } } as never;
    for (const m of [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, assetBrowser, actors, factions, conditions, combatTracker, sceneTools, chat, notes, tables, audio, vfx]) {
      m.register(ctx);
    }
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list[0]?.id).toBe("chat:panel");
  });

  it("the built default layout docks exactly chat:panel; every other panel starts closed in the launcher", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions, hooks: { on: () => () => {} } } as never;
    for (const m of [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, assetBrowser, actors, factions, conditions, combatTracker, sceneTools, chat, notes, tables, audio, vfx]) {
      m.register(ctx);
    }
    const regs = contributions.contributionsFor(PANEL_CONTRACT).map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement }));
    const layout = defaultLayout(regs);

    // Only chat:panel carries a defaultPlacement (docked right); every other panel has
    // none, so placeNewRegistrations records them in compact.order without placing them
    // in expanded — they start closed (reachable via the launcher), not minimized chips.
    const docked = Object.values(layout.expanded.zones).flatMap((z) => z.groups.flatMap((g) => g.tabs));
    expect(docked).toEqual(["chat:panel"]);
    expect(layout.expanded.minimized).toEqual([]);
    expect(layout.compact.order.sort()).toEqual(
      ["chat:panel", "asset-browser:panel", "actors:panel", "factions:panel", "conditions:panel", "combat-tracker:panel", "game-settings:panel", "settings:panel", "notes:panel", "tables:panel", "audio:panel", "vfx:panel"].sort(),
    );
  });
});

describe("sheet modules contribute sheets, not panels", () => {
  it("the six sheet modules register sheet contracts and no shadowcat.panel", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions, hooks: { on: () => () => {} } } as never;
    for (const m of [sheetFallback, sheetActor, sheetItem, sheetNote, sheetTable, sheetPlaylist]) m.register(ctx);
    expect(contributions.contributionsFor(PANEL_CONTRACT)).toHaveLength(0);
    expect(contributions.entriesFor(SHEET_FALLBACK_CONTRACT)).toHaveLength(1);
    expect(contributions.entriesFor(sheetContract("actor"))).toHaveLength(1);
    expect(contributions.entriesFor(sheetContract("item"))).toHaveLength(1);
    expect(contributions.entriesFor(sheetContract("note"))).toHaveLength(1);
    expect(contributions.entriesFor(sheetContract("table"))).toHaveLength(1);
    expect(contributions.entriesFor(sheetContract("playlist"))).toHaveLength(1);
  });
});

describe("ducking contributes a settings section, not a panel", () => {
  afterEach(() => localStorage.clear());

  it("registers exactly one shadowcat.settings-section entry and no shadowcat.panel entry", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions, hooks: { on: () => () => {} } } as never;
    ducking.register(ctx);
    expect(contributions.contributionsFor(PANEL_CONTRACT)).toHaveLength(0);
    expect(contributions.contributionsFor(SETTINGS_SECTION_CONTRACT)).toHaveLength(1);
    ducking.unregister?.();
  });
});

describe("dice3d contributes a stage overlay, not a panel", () => {
  it("registers exactly one shadowcat.stage-overlay contribution and no shadowcat.panel", () => {
    const contributions = new ContributionRegistry();
    const ctx = { contributions, hooks: { on: () => () => {} } } as never;
    dice3d.register(ctx);
    expect(contributions.contributionsFor(PANEL_CONTRACT)).toHaveLength(0);
    expect(contributions.contributionsFor(STAGE_OVERLAY_CONTRACT)).toHaveLength(1);
  });
});
