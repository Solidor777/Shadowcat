import { describe, it, expect, vi } from "vitest";
import { tick } from "svelte";
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
  buildActorDoc("w1", "G", { displayName: "G", visual: { kind: "image", asset: "a" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions, prototype: false, vision: null }, id);

describe("ConditionsPanel", () => {
  it("seeds the condition registry once on GM mount when absent", async () => {
    const dispatchIntent = vi.fn();
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }) });
    await vi.waitFor(() => expect(dispatchIntent).toHaveBeenCalled());
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("create");
    const doc = (ops[0] as { doc: WireDocument }).doc;
    expect(doc.doc_type).toBe("condition-registry");
    const conds = (doc.engine as { conditions: Record<string, unknown> }).conditions;
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
    expect(ops[0]).toMatchObject({ op: "update", doc_id: "act1", changes: [{ path: "/engine/conditions", new: ["dead"] }] });
  });

  it("reads the raw stored value as `old` on a SECOND edit to the same field (OCC regression)", async () => {
    const dispatchIntent = vi.fn();
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const store = storeWith(registry);
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }) });

    const nameInput = screen.getByLabelText("conditions.name") as HTMLInputElement;

    // First edit: pristine doc, `old` correctly matches the stored value.
    await fireEvent.change(nameInput, { target: { value: "Deceased" } });
    expect(dispatchIntent).toHaveBeenNthCalledWith(1, [
      { op: "update", doc_id: "creg1", changes: [{ path: "/engine/conditions/dead/name", old: "Dead", new: "Deceased" }] },
    ]);

    // Apply the first write to the store, as the server would on success, before the second edit.
    store.applyCommand({
      seq: 2, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "update", doc_id: "creg1", changes: [{ path: "/engine/conditions/dead/name", old: "Dead", new: "Deceased" }] }],
    });
    await tick();

    // Second edit to the SAME field: `old` must reflect the first write's result, not stay
    // hardcoded at null (a stale `old` gets rejected by the server's field-level OCC check).
    await fireEvent.change(nameInput, { target: { value: "Deceased2" } });
    expect(dispatchIntent).toHaveBeenNthCalledWith(2, [
      { op: "update", doc_id: "creg1", changes: [{ path: "/engine/conditions/dead/name", old: "Deceased", new: "Deceased2" }] },
    ]);
  });
});
