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

describe("CombatTrackerPanel reorder wiring", () => {
  function twoCombatantCombat(): { store: DocumentStore; combatId: string } {
    const store = new DocumentStore();
    const c = combatOn("scene-1", { id: "c1", active: true });
    const engineA: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    const engineB: CombatantEngine = { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
    const a = buildCombatantDoc("w1", "c1", engineA, { id: "a", name: "a" });
    const b = buildCombatantDoc("w1", "c1", engineB, { id: "b", name: "b" });
    (c.engine as { order: string[] }).order = ["a", "b"];
    store.applyCommand({ seq: 1, world_id: "w1", author: "x", ts: 0, ops: [{ op: "create", doc: c }, { op: "create", doc: a }, { op: "create", doc: b }] });
    return { store, combatId: "c1" };
  }

  it("a completed pointer drag past the other row's midpoint dispatches ONE reorder with moveInOrder's result", async () => {
    const { store, combatId } = twoCombatantCombat();
    const combat = fakeCombatApi(store, { role: "gm" });
    const { container } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm" }),
    });
    const rowEls = Array.from(container.querySelectorAll(".rows > div")) as HTMLElement[];
    expect(rowEls.length).toBe(2);
    rowEls[0].getBoundingClientRect = () => ({ top: 0, height: 40, bottom: 40, left: 0, right: 0, width: 0, x: 0, y: 0, toJSON: () => ({}) });
    rowEls[1].getBoundingClientRect = () => ({ top: 40, height: 40, bottom: 80, left: 0, right: 0, width: 0, x: 0, y: 40, toJSON: () => ({}) });
    const handle = rowEls[0].querySelector("button.drag-handle") as HTMLButtonElement;
    await fireEvent.pointerDown(handle, { clientY: 0 });
    await fireEvent(window, new PointerEvent("pointermove", { clientY: 70 }));
    await fireEvent(window, new PointerEvent("pointerup", { clientY: 70 }));
    expect(combat.calls.reorder).toEqual([[combatId, ["b", "a"]]]);
  });

  it("a two-step pointer drag (from 0 to 2 of three rows) discriminates argument order — moveInOrder(order, 0, 2) differs from moveInOrder(order, 2, 0)", async () => {
    const store = new DocumentStore();
    const c = combatOn("scene-1", { id: "c1", active: true });
    const mk = (id: string) => buildCombatantDoc("w1", "c1", { kind: { type: "actor", token_id: null, actor_id: null }, initiative: null, tiebreak: 0, resources: {} } as CombatantEngine, { id, name: id });
    (c.engine as { order: string[] }).order = ["a", "b", "c"];
    store.applyCommand({ seq: 1, world_id: "w1", author: "x", ts: 0, ops: [{ op: "create", doc: c }, { op: "create", doc: mk("a") }, { op: "create", doc: mk("b") }, { op: "create", doc: mk("c") }] });
    const combat = fakeCombatApi(store, { role: "gm" });
    const { container } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm" }),
    });
    const rowEls = Array.from(container.querySelectorAll(".rows > div")) as HTMLElement[];
    expect(rowEls.length).toBe(3);
    const stub = (top: number): DOMRect => ({ top, height: 40, bottom: top + 40, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({}) });
    rowEls[0].getBoundingClientRect = () => stub(0);
    rowEls[1].getBoundingClientRect = () => stub(40);
    rowEls[2].getBoundingClientRect = () => stub(80);
    const handle = rowEls[0].querySelector("button.drag-handle") as HTMLButtonElement;
    await fireEvent.pointerDown(handle, { clientY: 0 });
    await fireEvent(window, new PointerEvent("pointermove", { clientY: 110 })); // past both other midpoints (60, 100)
    await fireEvent(window, new PointerEvent("pointerup", { clientY: 110 }));
    // moveInOrder(["a","b","c"], 0, 2) => ["b","c","a"]; the swapped-argument bug would instead
    // produce moveInOrder(["a","b","c"], 2, 0) => ["c","a","b"] — the two are NOT equal, so this
    // assertion is direction-sensitive, unlike a one-step (adjacent) move.
    expect(combat.calls.reorder).toEqual([["c1", ["b", "c", "a"]]]);
  });

  it("Alt+ArrowDown on a focused row dispatches the one-step move", async () => {
    const { store, combatId } = twoCombatantCombat();
    const combat = fakeCombatApi(store, { role: "gm" });
    const { container } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "gm" }),
    });
    const rowEls = Array.from(container.querySelectorAll(".rows > div")) as HTMLElement[];
    await fireEvent.keyDown(rowEls[0], { key: "ArrowDown", altKey: true });
    expect(combat.calls.reorder).toEqual([[combatId, ["b", "a"]]]);
  });

  it("a player without edit affordance gets neither a drag handle nor the keyboard move", async () => {
    const { store } = twoCombatantCombat();
    const combat = fakeCombatApi(store, { role: "player" });
    combat.setCanAct({ edit: false });
    const { container } = render(CombatTrackerPanel, {
      props: { badge: new TurnBadge() },
      context: setAppContextForTest({ store, documents: store, combat, viewedSceneId: "scene-1", role: "player" }),
    });
    expect(container.querySelector("button.drag-handle")).toBeNull();
    const rowEls = Array.from(container.querySelectorAll(".rows > div")) as HTMLElement[];
    await fireEvent.keyDown(rowEls[0], { key: "ArrowDown", altKey: true });
    expect(combat.calls.reorder).toBeUndefined();
  });
});
