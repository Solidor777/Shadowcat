import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { TokenSelection } from "@shadowcat/ui-kit";
import {
  DocumentStore,
  buildActorDoc,
  buildConditionRegistryDoc,
  buildTokenFromActor,
  parseBars,
  type BarAttribute,
  type WireDocument,
} from "@shadowcat/core";
import TokenQuickEdit from "./TokenQuickEdit.svelte";

const leafBar: BarAttribute = { label: "HP", current: "hp.value", max: "hp.max", color: "#cc2222", mode: "always" };
const computedBar: BarAttribute = { label: "Temp", current: "hp.value + 2", max: 10, color: "#2222cc", mode: "always" };
const actorEngine = {
  displayName: "G", visual: { kind: "image" as const, asset: "a1" }, size: { w: 1, h: 1 },
  shape: "square" as const, faction: null as string | null, conditions: [] as string[], prototype: false,
  vision: null, light: null, movement: [], aura: null, sound: null, vfx: null,
  bars: [leafBar, computedBar],
};
const hpDelivered = { label: "HP", current: 7, max: 10, color: "#cc2222", mode: "always" };
const tempDelivered = { label: "Temp", current: 9, max: 10, color: "#2222cc", mode: "always" };

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A linked token on an actor whose `system` holds `hp`, plus the registry the palette lists. */
function setup(delivered: (typeof hpDelivered)[]) {
  const actor = buildActorDoc("w1", "G", actorEngine, "act1");
  actor.system = { hp: { value: 7, max: 10 } };
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀", overlay: false } }, "creg1");
  const tokenSelection = new TokenSelection();
  tokenSelection.set(["tok1"]);
  const bars = parseBars({ scenes: { scene1: { tokens: { tok1: { nameplate: "G", nameplate_mode: "always", bars: delivered } } } } });
  return { documents: storeWith(actor, token, registry), tokenSelection, bars };
}

describe("TokenQuickEdit", () => {
  it("renders nothing without a token", () => {
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([hpDelivered]), canEdit: () => true }), props: { tokenId: null } });
    expect(screen.queryByTestId("token-quick-edit")).toBeNull();
  });

  it("holds the elevation control, the condition palette and one row per delivered bar", () => {
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([hpDelivered, tempDelivered]), canEdit: () => true }), props: { tokenId: "tok1" } });
    expect(screen.getByTestId("token-elevation")).toBeTruthy();
    expect(screen.getByTitle("Dead")).toBeTruthy();
    expect(screen.getAllByTestId("quick-bar-row")).toHaveLength(2);
  });

  it("toggling a palette chip writes the actor's conditions with the raw stored old", async () => {
    const dispatchIntent = vi.fn();
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([]), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    await fireEvent.click(screen.getByTitle("Dead"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/engine/conditions", old: [], new: ["dead"] }] },
    ]);
  });

  it("the minus and plus buttons write the leaf with the delivered current value as old, by the step", async () => {
    const dispatchIntent = vi.fn();
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([hpDelivered]), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    await fireEvent.click(screen.getByTestId("quick-bar-dec"));
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/system/hp/value", old: 7, new: 6 }] },
    ]);
    await fireEvent.input(screen.getByTestId("quick-bar-step"), { target: { value: "5" } });
    await fireEvent.click(screen.getByTestId("quick-bar-inc"));
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/system/hp/value", old: 7, new: 12 }] },
    ]);
  });

  it("entering a value writes it, and text that is not a number writes nothing", async () => {
    const dispatchIntent = vi.fn();
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([hpDelivered]), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    await fireEvent.change(screen.getByTestId("quick-bar-value"), { target: { value: "3" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "act1", changes: [{ path: "/system/hp/value", old: 7, new: 3 }] },
    ]);
    dispatchIntent.mockClear();
    await fireEvent.change(screen.getByTestId("quick-bar-value"), { target: { value: "" } });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("a computed bar shows its value with disabled controls, a tooltip, and dispatches nothing", async () => {
    const dispatchIntent = vi.fn();
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([tempDelivered]), dispatchIntent, canEdit: () => true }), props: { tokenId: "tok1" } });
    const dec = screen.getByTestId("quick-bar-dec") as HTMLButtonElement;
    expect(dec.disabled).toBe(true);
    expect(dec.title).toBe("actors.quickBarComputed");
    expect((screen.getByTestId("quick-bar-value") as HTMLInputElement).value).toBe("9");
    await fireEvent.click(dec);
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("a bar the viewer may not edit is disabled with the read-only tooltip", () => {
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([hpDelivered]), canEdit: (_doc, path) => path === "/engine/elevation" }), props: { tokenId: "tok1" } });
    const inc = screen.getByTestId("quick-bar-inc") as HTMLButtonElement;
    expect(inc.disabled).toBe(true);
    expect(inc.title).toBe("actors.quickBarReadOnly");
  });

  it("a bar the channel does not deliver to this viewer has no row, though the actor authors it", () => {
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([tempDelivered]), canEdit: () => true }), props: { tokenId: "tok1" } });
    expect(screen.getAllByTestId("quick-bar-row")).toHaveLength(1);
    expect(screen.queryByText("HP")).toBeNull();
  });

  it("Delete token dispatches one delete op carrying the stored token as the full pre-image", async () => {
    const dispatchIntent = vi.fn();
    const s = setup([]);
    render(TokenQuickEdit, { context: setAppContextForTest({ ...s, dispatchIntent, canEdit: () => true, canDelete: () => true }), props: { tokenId: "tok1" } });
    const button = screen.getByTestId("token-delete") as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    await fireEvent.click(button);
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    expect(dispatchIntent).toHaveBeenCalledWith([{ op: "delete", doc: s.documents.get("tok1") }]);
  });

  it("shows no Delete token control when ctx.canDelete refuses the token, whatever canEdit says", () => {
    const seen: string[] = [];
    render(TokenQuickEdit, {
      context: setAppContextForTest({ ...setup([]), canEdit: () => true, canDelete: (doc) => (seen.push(doc.id), false) }),
      props: { tokenId: "tok1" },
    });
    expect(seen).toContain("tok1");
    expect(screen.queryByTestId("token-delete")).toBeNull();
  });

  it("shows no bar section when the channel delivers nothing for the token", () => {
    render(TokenQuickEdit, { context: setAppContextForTest({ ...setup([]), canEdit: () => true }), props: { tokenId: "tok1" } });
    expect(screen.queryByTestId("quick-bar-row")).toBeNull();
    expect(screen.queryByTestId("quick-bar-step")).toBeNull();
  });
});
