import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { TokenSelection } from "@shadowcat/ui-kit";
import { DocumentStore, buildActorDoc, buildTokenFromActor, buildConditionRegistryDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import ConditionsPanel from "./ConditionsPanel.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}
const actorDoc = (id: string, conditions: string[]) =>
  buildActorDoc("w1", { name: "G", displayName: "G", visual: { kind: "image", asset: "a" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions, prototype: false }, id);

describe("ConditionsPanel", () => {
  it("seeds the condition registry once on GM mount when absent", async () => {
    const dispatchIntent = vi.fn();
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }) });
    await vi.waitFor(() => expect(dispatchIntent).toHaveBeenCalled());
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("create");
    const doc = (ops[0] as { doc: WireDocument }).doc;
    expect(doc.doc_type).toBe("condition-registry");
    const conds = (doc.system as { conditions: Record<string, unknown> }).conditions;
    expect(Object.keys(conds).sort()).toEqual(["blinded", "dead", "hasted", "invisible", "poisoned", "prone", "slowed", "stunned", "unconscious"]);
  });

  it("does not re-seed when a registry already exists", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"));
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }) });
    await Promise.resolve();
    expect(dispatchIntent.mock.calls.some((c) => (c[0] as WireOperation[])[0]?.op === "create")).toBe(false);
  });

  it("does not toggle when the user may not edit the target (canEdit false)", async () => {
    const dispatchIntent = vi.fn();
    const actor = actorDoc("act1", []);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    const store = storeWith(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"), actor, token);
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ConditionsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, dispatchIntent, tokenSelection, canEdit: () => false }) });
    await fireEvent.click(screen.getByTitle("Dead"));
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("toggles the condition on the editable selected token (canEdit true)", async () => {
    const dispatchIntent = vi.fn();
    const actor = actorDoc("act1", []);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    const store = storeWith(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"), actor, token);
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ConditionsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, dispatchIntent, tokenSelection, canEdit: () => true }) });
    await fireEvent.click(screen.getByTitle("Dead"));
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({ op: "update", doc_id: "act1", changes: [{ path: "/system/conditions", new: ["dead"] }] });
  });
});
