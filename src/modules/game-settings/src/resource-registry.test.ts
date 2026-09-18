import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { i18n } from "@shadowcat/ui-kit";
import { DocumentStore, buildResourceRegistryDoc, type Resource, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

// Suppress listAssets fetch: the panel's dice-sound picker calls listAssets in an $effect
// which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

function registry(resources: Record<string, Resource> = {}): WireDocument {
  return buildResourceRegistryDoc("w1", resources, "rr1");
}

// Each resource row's controls are named by field AND key ("Max for resource hp"), which only the
// catalog-backed `t` renders — the fixture's identity-echo `t` drops interpolation params, so every
// row would share one name.
const t = (k: string, p?: Parameters<typeof i18n.t>[1]) => i18n.t(k, p);

describe("ResourceRegistryEditor", () => {
  it("renders entries in order, oldest-authored order field first", () => {
    const store = storeWith(registry({
      hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } },
      gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } },
    }));
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent: vi.fn(), t }) });
    const names = screen.getAllByLabelText(/^Name for resource /).map((el) => (el as HTMLInputElement).value);
    expect(names).toEqual(["Gold", "HP"]);
  });

  it("editing name dispatches a field write with the raw pre-image", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent, t }) });
    const name = screen.getByLabelText("Name for resource gold") as HTMLInputElement;
    await fireEvent.change(name, { target: { value: "Coins" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/name", old: "Gold", new: "Coins" }] },
    ]);
  });

  it("editing order dispatches a field write", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent, t }) });
    const order = screen.getByLabelText("Order for resource gold") as HTMLInputElement;
    await fireEvent.change(order, { target: { value: "5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/order", old: 0, new: 5 }] },
    ]);
  });

  it("a mirror value of \"30\" writes the number 30", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent, t }) });
    const value = screen.getByLabelText("Value for resource gold") as HTMLInputElement;
    await fireEvent.change(value, { target: { value: "30" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/gold/binding/value", old: 0, new: 30 }] },
    ]);
  });

  it("a tracked max of \"speed\" writes the string; \"1 +\" shows an inline error and writes nothing", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ hp: { name: "HP", order: 0, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } })), dispatchIntent, t }) });
    const max = screen.getByLabelText("Max for resource hp") as HTMLInputElement;
    await fireEvent.change(max, { target: { value: "speed" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/hp/binding/max", old: 20, new: "speed" }] },
    ]);
    dispatchIntent.mockClear();
    await fireEvent.change(max, { target: { value: "1 +" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText(/^Invalid formula: /)).toBeTruthy();
  });

  it("a recovery field edit dispatches a field write at the snake_case wire path", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ hp: { name: "HP", order: 0, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } } })), dispatchIntent, t }) });
    const turnStart = screen.getByLabelText("Turn start for resource hp") as HTMLInputElement;
    await fireEvent.change(turnStart, { target: { value: "5" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "rr1", changes: [{ path: "/engine/resources/hp/binding/recover/turn_start", old: 0, new: 5 }] },
    ]);
  });

  it("switching kind dispatches ONE update at the binding path with the new kind's defaults, preserving name/order", async () => {
    const dispatchIntent = vi.fn();
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 3, binding: { kind: "mirror", value: 7 } } })), dispatchIntent, t }) });
    const kind = screen.getByLabelText("Kind for resource gold") as HTMLSelectElement;
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
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent, t }) });
    const key = screen.getByLabelText("Key") as HTMLInputElement;
    const add = screen.getByRole("button", { name: "Add resource" });

    await fireEvent.change(key, { target: { value: "Bad Key!" } });
    await fireEvent.click(add);
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText(/^Keys must start with a lowercase letter/)).toBeTruthy();

    await fireEvent.change(key, { target: { value: "gold" } });
    await fireEvent.click(add);
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(screen.getByText("That key is already in use.")).toBeTruthy();

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
        role: "gm", world: "w1", dispatchIntent, t,
        documents: storeWith(registry({
          gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } },
          hp: { name: "HP", order: 1, binding: { kind: "tracked", max: 20, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } } },
        })),
      }),
    });
    const removeGold = screen.getByRole("button", { name: "Remove resource gold" });
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
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: storeWith(registry({ gold: { name: "Gold", order: 0, binding: { kind: "mirror", value: 0 } } })), dispatchIntent: vi.fn(), t }) });
    expect(screen.queryByText("Resources")).toBeNull();
  });
});
