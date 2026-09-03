import { describe, it, expect, vi } from "vitest";
import { TurnBadge } from "./turnBadge";
import type { CombatTurnEvent } from "@shadowcat/core";

function turnEvent(combatantId: string): CombatTurnEvent {
  return { combatId: "combat-1", round: 1, combatantId, kind: "actor" };
}

describe("TurnBadge", () => {
  it("stays at 0 and never notifies before bind() is called", () => {
    const badge = new TurnBadge();
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.onTurnStart(turnEvent("mine"));
    expect(badge.get()).toBe(0);
    expect(listener).not.toHaveBeenCalled();
  });

  it("counts 1 and notifies once when bound and the started combatant is mine", () => {
    const badge = new TurnBadge();
    const notify = vi.fn();
    badge.bind((id) => id === "mine", notify);
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.onTurnStart(turnEvent("mine"));
    expect(badge.get()).toBe(1);
    expect(notify).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("stays at 0 and does not notify when the started combatant is not mine", () => {
    const badge = new TurnBadge();
    const notify = vi.fn();
    badge.bind((id) => id === "mine", notify);
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.onTurnStart(turnEvent("someone-else"));
    expect(badge.get()).toBe(0);
    expect(notify).not.toHaveBeenCalled();
    expect(listener).not.toHaveBeenCalled();
  });

  it("resets to 0 on turn-end naming the current combatant", () => {
    const badge = new TurnBadge();
    badge.bind((id) => id === "mine", vi.fn());
    badge.onTurnStart(turnEvent("mine"));
    expect(badge.get()).toBe(1);
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.onTurnEnd(turnEvent("mine"));
    expect(badge.get()).toBe(0);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("clear() resets to 0", () => {
    const badge = new TurnBadge();
    badge.bind((id) => id === "mine", vi.fn());
    badge.onTurnStart(turnEvent("mine"));
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.clear();
    expect(badge.get()).toBe(0);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("subscribe listeners fire only when the count actually changes", () => {
    const badge = new TurnBadge();
    badge.bind((id) => id === "mine", vi.fn());
    const listener = vi.fn();
    badge.subscribe(listener);
    badge.onTurnEnd(turnEvent("mine")); // already 0 -> 0, no change
    expect(listener).not.toHaveBeenCalled();
    badge.clear(); // already 0 -> 0, no change
    expect(listener).not.toHaveBeenCalled();
  });

  it("subscribe returns an unsubscribe function", () => {
    const badge = new TurnBadge();
    badge.bind((id) => id === "mine", vi.fn());
    const listener = vi.fn();
    const unsubscribe = badge.subscribe(listener);
    unsubscribe();
    badge.onTurnStart(turnEvent("mine"));
    expect(listener).not.toHaveBeenCalled();
  });
});
