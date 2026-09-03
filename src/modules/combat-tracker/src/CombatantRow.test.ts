import { describe, it, expect, vi, afterEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildCombatantDoc, buildChannelRegistryDoc, type CombatantEngine, type CombatAffordances, type Resource } from "@shadowcat/core";
import CombatantRow from "./CombatantRow.svelte";
import { fakeCombatApi } from "./__fixtures__/fakeCombatApi";
import type { Row } from "./model";

afterEach(() => cleanup());

const FULL_CAN: CombatAffordances = {
  start: true, pause: true, end: true, advance: true, rewind: true, sort: true, edit: true,
  roll: () => true, resource: () => true,
};

function actorRow(id: string, opts: { initiative?: number | null; tiebreak?: number; owner?: string | null; hidden?: boolean; view?: Row["view"] } = {}): Row {
  const engine: CombatantEngine = {
    kind: { type: "actor", token_id: null, actor_id: null },
    initiative: opts.initiative ?? null,
    tiebreak: opts.tiebreak ?? 0,
    resources: {},
  };
  const doc = buildCombatantDoc("w1", "combat-1", engine, { id, owner: opts.owner, hidden: opts.hidden, name: id });
  return { doc, kind: "actor", view: opts.view ?? null, name: id, art: {} };
}

function eventRow(id: string, lifespan: number | null, message: string | null): Row {
  const engine: CombatantEngine = { kind: { type: "event", lifespan, message }, initiative: null, tiebreak: 0, resources: {} };
  const doc = buildCombatantDoc("w1", "combat-1", engine, { id, name: "An event" });
  return { doc, kind: "event", view: null, name: "An event", art: {} };
}

function renderRow(row: Row, opts: { can?: Partial<CombatAffordances>; registry?: [string, Resource][]; isTurn?: boolean; busy?: boolean; onDragStart?: (i: number, e: PointerEvent) => void; openDocument?: (ref: unknown) => void } = {}) {
  const store = new DocumentStore();
  const combat = fakeCombatApi(store);
  return {
    ...render(CombatantRow, {
      props: {
        row, combatId: "combat-1", registry: opts.registry ?? [],
        isTurn: opts.isTurn ?? false, can: { ...FULL_CAN, ...opts.can }, busy: opts.busy ?? false,
        run: async (fn: () => Promise<void>) => fn(),
        notation: "1d20",
        onDragStart: opts.onDragStart ?? (() => {}),
        index: 0,
      },
      context: setAppContextForTest({ store, documents: store, combat, openDocument: opts.openDocument as never }),
    }),
    combat,
  };
}

describe("CombatantRow", () => {
  it("marks the current-turn row with aria-current", () => {
    const { getByTestId } = renderRow(actorRow("a"), { isTurn: true });
    expect(getByTestId("combat-tracker:row-a").getAttribute("aria-current")).toBe("true");
  });

  it("falls back to the unnamed label when name is null (a redacted name)", () => {
    const row = actorRow("a");
    row.name = null;
    const { getByText } = renderRow(row);
    expect(getByText("combatTracker.unnamed")).toBeTruthy();
  });

  it("opens the sheet with a tokenId ref on name click when a token resolves", async () => {
    const row = actorRow("a");
    row.art = { tokenId: "token-1" };
    const openDocument = vi.fn();
    const { getByText } = renderRow(row, { openDocument });
    await fireEvent.click(getByText("a"));
    expect(openDocument).toHaveBeenCalledWith({ tokenId: "token-1" });
  });

  it("initiative input dispatches setInitiative", async () => {
    const { getByLabelText, combat } = renderRow(actorRow("a"));
    await fireEvent.change(getByLabelText("combatTracker.initiative"), { target: { value: "12" } });
    expect(combat.calls.setInitiative).toEqual([["a", 12, undefined]]);
  });

  it("clearing initiative dispatches null", async () => {
    const { getByLabelText, combat } = renderRow(actorRow("a", { initiative: 5 }));
    await fireEvent.change(getByLabelText("combatTracker.initiative"), { target: { value: "" } });
    expect(combat.calls.setInitiative).toEqual([["a", null, undefined]]);
  });

  it("per-row roll dispatches roll with one entry", async () => {
    const store = new DocumentStore();
    store.applyCommand({
      seq: 1, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "create", doc: buildChannelRegistryDoc("w1", { general: { name: "General" } }) }],
    });
    const combat = fakeCombatApi(store);
    const { getByTestId } = render(CombatantRow, {
      props: {
        row: actorRow("a"), combatId: "combat-1", registry: [],
        isTurn: false, can: FULL_CAN, busy: false,
        run: async (fn: () => Promise<void>) => fn(),
        notation: "1d20", onDragStart: () => {}, index: 0,
      },
      context: setAppContextForTest({ store, documents: store, combat }),
    });
    await fireEvent.click(getByTestId("combat-tracker:roll-a"));
    expect(combat.calls.roll).toEqual([["combat-1", "general", [{ combatant_id: "a", notation: "1d20" }]]]);
  });

  it("stepper and direct entry dispatch modifyResource with the delta/set shapes", async () => {
    const tracked: Resource = { name: "HP", order: 0, binding: { kind: "tracked", max: 10, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } };
    const row = actorRow("a", { view: { id: "a", resources: { hp: { binding: "tracked", current: 5, max: 10, error: null } }, movementCells: null } });
    const { getByTestId, combat } = renderRow(row, { registry: [["hp", tracked]] });
    const cell = getByTestId("combat-tracker:resource-hp");
    const minus = cell.querySelector("button:first-of-type") as HTMLButtonElement;
    await fireEvent.click(minus);
    expect(combat.calls.modifyResource).toEqual([["combat-1", "a", "hp", { kind: "delta", amount: -1 }]]);
    const input = cell.querySelector("input") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "7" } });
    expect(combat.calls.modifyResource![1]).toEqual(["combat-1", "a", "hp", { kind: "set", value: 7 }]);
  });

  it("renders a Mirror resource read-only", () => {
    const mirror: Resource = { name: "Morale", order: 0, binding: { kind: "mirror", value: 3 } };
    const row = actorRow("a", { view: { id: "a", resources: { morale: { binding: "mirror", current: 3, max: null, error: null } }, movementCells: null } });
    const { getByTestId } = renderRow(row, { registry: [["morale", mirror]] });
    const cell = getByTestId("combat-tracker:resource-morale");
    expect(cell.querySelector("input")).toBeNull();
    expect(cell.querySelector("button")).toBeNull();
    expect(cell.textContent?.trim()).toBe("3");
  });

  it("renders the error glyph on an evaluation error", () => {
    const tracked: Resource = { name: "HP", order: 0, binding: { kind: "tracked", max: 10, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } };
    const row = actorRow("a", { view: { id: "a", resources: { hp: { binding: "tracked", current: null, max: null, error: "bad" } }, movementCells: null } });
    const { getByTestId } = renderRow(row, { registry: [["hp", tracked]] });
    expect(getByTestId("combat-tracker:resource-hp").textContent?.trim()).toBe("⚠");
  });

  it("renders a blank resource cell when the row's view is null", () => {
    const tracked: Resource = { name: "HP", order: 0, binding: { kind: "tracked", max: 10, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } };
    const { getByTestId } = renderRow(actorRow("a", { view: null }), { registry: [["hp", tracked]] });
    expect(getByTestId("combat-tracker:resource-hp").textContent?.trim()).toBe("");
  });

  it("renders an event row's lifespan and message, with no resource cells or roll button", () => {
    const tracked: Resource = { name: "HP", order: 0, binding: { kind: "tracked", max: 10, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } };
    const row = eventRow("e1", 3, "The floor shakes");
    const { getByText, queryByTestId } = renderRow(row, { registry: [["hp", tracked]] });
    expect(getByText("3")).toBeTruthy();
    expect(getByText("The floor shakes")).toBeTruthy();
    expect(queryByTestId("combat-tracker:resource-hp")).toBeNull();
    expect(queryByTestId("combat-tracker:roll-e1")).toBeNull();
  });

  it("renders infinity for an event with no lifespan", () => {
    const row = eventRow("e1", null, null);
    const { getByText } = renderRow(row);
    expect(getByText("∞")).toBeTruthy();
  });

  it("hidden toggle both directions dispatch setHidden", async () => {
    const notHidden = actorRow("a", { hidden: false });
    const { getByText, combat } = renderRow(notHidden);
    await fireEvent.click(getByText("combatTracker.hidden"));
    expect(combat.calls.setHidden).toEqual([["a", true]]);
  });

  it("reveal dispatches setHidden(false)", async () => {
    const hidden = actorRow("a", { hidden: true });
    const { getByText, combat } = renderRow(hidden);
    await fireEvent.click(getByText("combatTracker.visible"));
    expect(combat.calls.setHidden).toEqual([["a", false]]);
  });

  it("remove dispatches removeCombatant, and is disabled with a hint on the current turn", async () => {
    const { getByText, combat } = renderRow(actorRow("a"));
    await fireEvent.click(getByText("combatTracker.remove"));
    expect(combat.calls.removeCombatant).toEqual([["combat-1", "a"]]);
  });

  it("remove is disabled on the row currently taking its turn", () => {
    const { getByText } = renderRow(actorRow("a"), { isTurn: true });
    const button = getByText("combatTracker.remove") as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.title).toBe("combatTracker.removeTurnHint");
  });

  it("hides GM-only controls (drag handle, hidden toggle, remove) for a non-editing player", () => {
    const { queryByText, queryByLabelText } = renderRow(actorRow("a"), { can: { edit: false } });
    expect(queryByLabelText("combatTracker.dragHandle")).toBeNull();
    expect(queryByText("combatTracker.remove")).toBeNull();
    expect(queryByText("combatTracker.hidden")).toBeNull();
  });

  it("renders the drag handle only when can.edit", () => {
    const { getByLabelText } = renderRow(actorRow("a"), { can: { edit: true } });
    expect(getByLabelText("combatTracker.dragHandle")).toBeTruthy();
  });
});
