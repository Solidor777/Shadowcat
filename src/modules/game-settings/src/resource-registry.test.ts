import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildResourceRegistryDoc, type Resource, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

function registry(resources: Record<string, Resource> = {}): WireDocument {
  return buildResourceRegistryDoc("w1", resources, "rr1");
}

describe("ResourceRegistryEditor", () => {
  it("renders entries in order, oldest-authored order field first", () => {
    const store = storeWith(registry({
      hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } },
      gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } },
    }));
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn() }) });
    const names = screen.getAllByLabelText(/gameSettings\.resources\.name-/).map((el) => (el as HTMLInputElement).value);
    expect(names).toEqual(["Gold", "HP"]);
  });

  it("editing name dispatches a field write with the raw pre-image", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent }) });
    const name = screen.getByLabelText("gameSettings.resources.name-gold") as HTMLInputElement;
    await fireEvent.change(name, { target: { value: "Coins" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/name", old: "Gold", new: "Coins" }] },
    ]);
  });

  it("editing order dispatches a field write", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent }) });
    const order = screen.getByLabelText("gameSettings.resources.order-gold") as HTMLInputElement;
    await fireEvent.change(order, { target: { value: "5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/order", old: 0, new: 5 }] },
    ]);
  });

  it("a mirror value of \"30\" writes the number 30", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent }) });
    const value = screen.getByLabelText("gameSettings.resources.value-gold") as HTMLInputElement;
    await fireEvent.change(value, { target: { value: "30" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/binding/value", old: 0, new: 30 }] },
    ]);
  });

  it("a tracked max of \"speed\" writes the string; \"1 +\" shows an inline error and writes nothing", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ hp: { name: "HP", order: 0, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } })), dispatchIntent }) });
    const max = screen.getByLabelText("gameSettings.resources.max-hp") as HTMLInputElement;
    await fireEvent.change(max, { target: { value: "speed" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/hp/binding/max", old: 20, new: "speed" }] },
    ]);
    dispatchIntent.mockClear();
    await fireEvent.change(max, { target: { value: "1 +" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("gameSettings.resources.invalid")).toBeTruthy();
  });

  it("a recovery field edit dispatches a field write at the snake_case wire path", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ hp: { name: "HP", order: 0, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } })), dispatchIntent }) });
    const turnStart = screen.getByLabelText("gameSettings.resources.turnStart-hp") as HTMLInputElement;
    await fireEvent.change(turnStart, { target: { value: "5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/hp/binding/recover/turn_start", old: 0, new: 5 }] },
    ]);
  });

  it("switching kind dispatches ONE update at the binding path with the new kind's defaults, preserving name/order", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 3, binding: { kind: "mirror", value: 7 } } })), dispatchIntent }) });
    const kind = screen.getByLabelText("gameSettings.resources.kind-gold") as HTMLSelectElement;
    await fireEvent.change(kind, { target: { value: "tracked" } });
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "rr1",
        changes: [{
          path: "/engine/resources/gold/binding",
          old: { kind: "mirror", value: 7 },
          new: { kind: "tracked", max: 0, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } },
        }],
      },
    ]);
  });

  it("add validates the key shape and uniqueness, then writes the whole entry at old: null", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent }) });
    const key = screen.getByLabelText("gameSettings.resources.key") as HTMLInputElement;
    const add = screen.getByRole("button", { name: "gameSettings.resources.add" });

    await fireEvent.change(key, { target: { value: "Bad Key!" } });
    await fireEvent.click(add);
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("gameSettings.resources.keyShape")).toBeTruthy();

    await fireEvent.change(key, { target: { value: "gold" } });
    await fireEvent.click(add);
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("gameSettings.resources.keyTaken")).toBeTruthy();

    await fireEvent.change(key, { target: { value: "mana" } });
    await fireEvent.click(add);
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/mana", old: null, new: { name: "mana", order: 1, binding: { kind: "mirror", value: 0 } } }] },
    ]);
  });

  it("remove rewrites the whole map without the removed key", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, {
      context: setAppContextForTest({
        role: "gm", world: "w1", dispatchIntent,
        documents: storeWith(registry({
          gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } },
          hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } },
        })),
      }),
    });
    const removeGold = screen.getByRole("button", { name: "gameSettings.resources.remove-gold" });
    await fireEvent.click(removeGold);
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "rr1",
        changes: [{
          path: "/engine/resources",
          old: {
            gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } },
            hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } },
          },
          new: { hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } },
        }],
      },
    ]);
  });

  it("renders nothing for a non-GM", () => {
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent: vi.fn() }) });
    expect(screen.queryByText("gameSettings.resources.title")).toBeNull();
  });
});
