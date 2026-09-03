import { describe, it, expect, vi } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { combatTracker } from "./index";
import { TurnBadge } from "./turnBadge";

/** A `ModuleContext.hooks`-shaped fake that records every `on` registration. */
function fakeHooks(): { on: ReturnType<typeof vi.fn>; calls: { name: string; handler: (p: unknown) => unknown }[] } {
  const calls: { name: string; handler: (p: unknown) => unknown }[] = [];
  const on = vi.fn((name: string, handler: (p: unknown) => unknown) => {
    calls.push({ name, handler });
    return () => {};
  });
  return { on, calls };
}

describe("combatTracker module", () => {
  it("declares its manifest and requires the panel surface", () => {
    expect(combatTracker.manifest.id).toBe("combat-tracker");
    expect(combatTracker.manifest.requires).toContain(PANEL_CONTRACT);
    expect(combatTracker.manifest.provides).toEqual([]);
  });

  it("contributes an order-2 launcher-closed panel with a badge", () => {
    const contributions = new ContributionRegistry();
    const hooks = fakeHooks();
    combatTracker.register({ contributions, hooks } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].id).toBe("combat-tracker:panel");
    expect(list[0].order).toBe(2);
    expect(list[0].panel?.icon).toBe("⚔️");
    expect(list[0].panel?.labelKey).toBe("combatTracker.tab");
    expect(list[0].panel?.defaultPlacement).toBeUndefined();
    expect(list[0].panel?.badge).toBeInstanceOf(TurnBadge);
    expect(list[0].props?.badge).toBe(list[0].panel?.badge);
  });

  it("registers the three turn-start/turn-end/end hook listeners", () => {
    const contributions = new ContributionRegistry();
    const hooks = fakeHooks();
    combatTracker.register({ contributions, hooks } as never);
    expect(hooks.calls.map((c) => c.name)).toEqual(["combat:turn-start", "combat:turn-end", "combat:end"]);
  });

  it("wires the registered listeners to the same badge instance the contribution exposes", () => {
    const contributions = new ContributionRegistry();
    const hooks = fakeHooks();
    combatTracker.register({ contributions, hooks } as never);
    const badge = contributions.contributionsFor(PANEL_CONTRACT)[0].panel?.badge as TurnBadge;
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.bind((id) => id === "mine", vi.fn());
    hooks.calls.find((c) => c.name === "combat:turn-start")!.handler({ combatId: "c1", round: 1, combatantId: "mine", kind: "actor" });
    expect(badge.get()).toBe(1);
    expect(listener).toHaveBeenCalledTimes(1);
    hooks.calls.find((c) => c.name === "combat:turn-end")!.handler({ combatId: "c1", round: 1, combatantId: "mine", kind: "actor" });
    expect(badge.get()).toBe(0);
    hooks.calls.find((c) => c.name === "combat:turn-start")!.handler({ combatId: "c1", round: 1, combatantId: "mine", kind: "actor" });
    expect(badge.get()).toBe(1);
    hooks.calls.find((c) => c.name === "combat:end")!.handler(undefined);
    expect(badge.get()).toBe(0);
  });
});
