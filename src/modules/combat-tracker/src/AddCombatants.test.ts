import { describe, it, expect, afterEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import { TokenSelection } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildCombatantDoc, type CombatantEngine } from "@shadowcat/core";
import AddCombatants from "./AddCombatants.svelte";
import { fakeCombatApi } from "./__fixtures__/fakeCombatApi";
import type { Row } from "./model";

afterEach(() => cleanup());

function actorRow(id: string, tokenId: string | null): Row {
  const engine: CombatantEngine = { kind: { type: "actor", token_id: tokenId, actor_id: null }, initiative: null, tiebreak: 0, resources: {} };
  return { doc: buildCombatantDoc("w1", "combat-1", engine, { id }), kind: "actor", view: null, name: id, art: {} };
}

function renderAdd(rows: Row[], selectedTokenIds: string[]) {
  const store = new DocumentStore();
  const combat = fakeCombatApi(store);
  const tokenSelection = new TokenSelection();
  tokenSelection.set(selectedTokenIds);
  return {
    ...render(AddCombatants, {
      props: { combatId: "combat-1", rows },
      context: setAppContextForTest({ store, documents: store, combat, tokenSelection }),
    }),
    combat,
  };
}

describe("AddCombatants", () => {
  it("counts only selected tokens NOT already in the combat", () => {
    const rows = [actorRow("a", "token-1")];
    const { getByTestId } = renderAdd(rows, ["token-1", "token-2"]);
    expect(getByTestId("combat-tracker:add-selected").textContent).toBe("combatTracker.addSelected");
  });

  it("clicking Add selected calls addCombatants with the filtered token set", async () => {
    const rows = [actorRow("a", "token-1")];
    const { getByTestId, combat } = renderAdd(rows, ["token-1", "token-2"]);
    await fireEvent.click(getByTestId("combat-tracker:add-selected"));
    expect(combat.calls.addCombatants).toEqual([["combat-1", [{ tokenId: "token-2", hidden: false }]]]);
  });

  it("the hidden checkbox applies to the whole added batch", async () => {
    const { getByTestId, container, combat } = renderAdd([], ["token-1", "token-2"]);
    const checkbox = container.querySelector("input[type=checkbox]") as HTMLInputElement;
    await fireEvent.click(checkbox);
    await fireEvent.click(getByTestId("combat-tracker:add-selected"));
    expect(combat.calls.addCombatants).toEqual([["combat-1", [{ tokenId: "token-1", hidden: true }, { tokenId: "token-2", hidden: true }]]]);
  });

  it("is disabled when there are no addable selected tokens", () => {
    const { getByTestId } = renderAdd([], []);
    expect((getByTestId("combat-tracker:add-selected") as HTMLButtonElement).disabled).toBe(true);
  });

  it("the event form requires a name", async () => {
    const { getByText, getByTestId, combat } = renderAdd([], []);
    await fireEvent.click(getByText("combatTracker.addEvent"));
    await fireEvent.click(getByTestId("combat-tracker:add-event"));
    expect(combat.calls.addEvent).toBeUndefined();
  });

  it("a blank lifespan parses to null (infinite)", async () => {
    const { getByText, getByLabelText, getByTestId, combat } = renderAdd([], []);
    await fireEvent.click(getByText("combatTracker.addEvent"));
    await fireEvent.input(getByLabelText("combatTracker.eventName"), { target: { value: "A rumble" } });
    await fireEvent.click(getByTestId("combat-tracker:add-event"));
    expect(combat.calls.addEvent).toEqual([["combat-1", { name: "A rumble", lifespan: null, message: null, hidden: false }]]);
  });

  it("submitting the event form calls addEvent with the parsed lifespan and message", async () => {
    const { getByText, getByLabelText, getByTestId, combat } = renderAdd([], []);
    await fireEvent.click(getByText("combatTracker.addEvent"));
    await fireEvent.input(getByLabelText("combatTracker.eventName"), { target: { value: "The floor shakes" } });
    await fireEvent.input(getByLabelText("combatTracker.eventLifespan"), { target: { value: "3" } });
    await fireEvent.input(getByLabelText("combatTracker.eventMessage"), { target: { value: "Rumble!" } });
    await fireEvent.click(getByTestId("combat-tracker:add-event"));
    expect(combat.calls.addEvent).toEqual([["combat-1", { name: "The floor shakes", lifespan: 3, message: "Rumble!", hidden: false }]]);
  });
});
