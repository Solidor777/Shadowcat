import { describe, it, expect, vi } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import { afterEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildCombatDoc, buildCombatantDoc, newCombatEngine, type CombatantEngine } from "@shadowcat/core";
import CombatTrackerPanel from "./CombatTrackerPanel.svelte";
import { TurnBadge } from "./turnBadge";
import { fakeCombatApi } from "./__fixtures__/fakeCombatApi";

afterEach(() => cleanup());

function combatOn(sceneId: string, opts: { id?: string; active?: boolean } = {}) {
  const engine = newCombatEngine(sceneId);
  if (opts.active) engine.active = true;
  return buildCombatDoc("w1", engine, opts.id);
}

describe("CombatTrackerPanel scoping/picker/create", () => {
  it("lists only the viewed scene's combats", () => {
    const store = new DocumentStore();
    const inScene = combatOn("scene-1", { id: "c1" });
    const otherScene = combatOn("scene-2", { id: "c2" });
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: inScene }, { op: "create", doc: otherScene }] });
    const combat = fakeCombatApi(store);
    const badge = new TurnBadge();
    const { getByTestId, queryByLabelText } = render(CombatTrackerPanel, {
      props: { badge },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1" }),
    });
    expect(getByTestId("combat-tracker:selected").textContent).toBe("c1");
    expect(queryByLabelText("combatTracker.pick")).toBeNull();
  });

  it("defaults to the active combat (combatsFor's own active-first order) when more than one exists", () => {
    const store = new DocumentStore();
    const inactive = combatOn("scene-1", { id: "c1", active: false });
    const active = combatOn("scene-1", { id: "c2", active: true });
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: inactive }, { op: "create", doc: active }] });
    const combat = fakeCombatApi(store);
    const { getByTestId } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1" }),
    });
    expect(getByTestId("combat-tracker:selected").textContent).toBe("c2");
  });

  it("shows a picker with two combats on one scene, and switching it changes the selection", async () => {
    const store = new DocumentStore();
    const a = combatOn("scene-1", { id: "c1" });
    const b = combatOn("scene-1", { id: "c2" });
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: a }, { op: "create", doc: b }] });
    const combat = fakeCombatApi(store);
    const { getByLabelText, getByTestId } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1" }),
    });
    const select = getByLabelText("combatTracker.pick") as HTMLSelectElement;
    expect(getByTestId("combat-tracker:selected").textContent).toBe("c1");
    await fireEvent.change(select, { target: { value: "c2" } });
    expect(getByTestId("combat-tracker:selected").textContent).toBe("c2");
  });

  it("GM with no combats sees a create button that calls createCombat(sceneId)", async () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store);
    const { getByTestId } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm" }),
    });
    await fireEvent.click(getByTestId("combat-tracker:create"));
    expect(combat.calls.createCombat).toEqual([["scene-1", {}]]);
  });

  it("a player with no combats sees the player hint, not a create button", () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store, { role: "player" });
    const { getByText, queryByTestId } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "player" }),
    });
    expect(queryByTestId("combat-tracker:create")).toBeNull();
    expect(getByText("combatTracker.noCombatPlayer")).toBeTruthy();
  });

  it("binds the badge on mount so an own combat:turn-start increments it and notifies", () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store, { selfId: "player-1", role: "player" });
    const badge = new TurnBadge();
    const notify = vi.fn();
    render(CombatTrackerPanel, {
      props: { badge },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "player", selfId: "player-1", notify }),
    });
    const combatantEngine: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: buildCombatantDoc("w1", "combat-1", combatantEngine, { id: "own-combatant", owner: "player-1" }) }],
    });
    badge.onTurnStart({ combatId: "c1", round: 1, combatantId: "own-combatant", kind: "actor" });
    expect(badge.get()).toBe(1);
    expect(notify).toHaveBeenCalledTimes(1);
  });

  it("does not increment or notify for a combatant that is not the bound player's own", () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store, { selfId: "player-1", role: "player" });
    const badge = new TurnBadge();
    const notify = vi.fn();
    render(CombatTrackerPanel, {
      props: { badge },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "player", selfId: "player-1", notify }),
    });
    const combatantEngine: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: buildCombatantDoc("w1", "combat-1", combatantEngine, { id: "someone-elses-combatant", owner: "player-2" }) }],
    });
    badge.onTurnStart({ combatId: "c1", round: 1, combatantId: "someone-elses-combatant", kind: "actor" });
    expect(badge.get()).toBe(0);
    expect(notify).not.toHaveBeenCalled();
  });

  it("does not bind the notice for a GM viewer, even for a combatant they own", () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store, { selfId: "gm-1", role: "gm" });
    const badge = new TurnBadge();
    const notify = vi.fn();
    render(CombatTrackerPanel, {
      props: { badge },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm", selfId: "gm-1", notify }),
    });
    const combatantEngine: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: buildCombatantDoc("w1", "combat-1", combatantEngine, { id: "gm-owned", owner: "gm-1" }) }],
    });
    badge.onTurnStart({ combatId: "c1", round: 1, combatantId: "gm-owned", kind: "actor" });
    expect(badge.get()).toBe(0);
    expect(notify).not.toHaveBeenCalled();
  });

  it("a rejected create surfaces through notify and resets busy", async () => {
    const store = new DocumentStore();
    const combat = fakeCombatApi(store);
    combat.rejectNext("createCombat", "no such scene");
    const notify = vi.fn();
    const { getByTestId } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm", notify }),
    });
    const button = getByTestId("combat-tracker:create") as HTMLButtonElement;
    await fireEvent.click(button);
    expect(notify).toHaveBeenCalledWith("no such scene", "warning");
    expect(button.disabled).toBe(false);
  });
});
