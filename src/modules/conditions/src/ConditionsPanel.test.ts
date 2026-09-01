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
  buildActorDoc("w1", "G", { displayName: "G", visual: { kind: "image", asset: "a" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions, prototype: false, vision: null, aura: null, sound: null, vfx: null }, id);

describe("ConditionsPanel", () => {
  it("never dispatches a registry create on GM mount (the server seeds it at world creation/join)", async () => {
    const dispatchIntent = vi.fn();
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent }) });
    await Promise.resolve();
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("does not dispatch a create when a registry already exists either", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"));
    render(ConditionsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }) });
    await Promise.resolve();
    expect(dispatchIntent.mock.calls.some((c) => (c[0] as WireOperation[])[0]?.op === "create")).toBe(false);
  });

  it("does not toggle when the user may not edit the target (canEdit false)", async () => {
    const dispatchIntent = vi.fn();
    const actor = actorDoc("act1", []);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
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
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    const store = storeWith(buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"), actor, token);
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok1"]);
    render(ConditionsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, dispatchIntent, tokenSelection, canEdit: () => true }) });
    await fireEvent.click(screen.getByTitle("Dead"));
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({ op: "update", doc_id: "act1", changes: [{ path: "/engine/conditions", new: ["dead"] }] });
  });

  it("isActive counts only editable targets: an editable token uniformly has it, a non-editable token in the same selection lacks it", async () => {
    const dispatchIntent = vi.fn();
    const editableActor = actorDoc("act-editable", ["dead"]);
    const nonEditableActor = actorDoc("act-non-editable", []);
    const editableToken = buildTokenFromActor("w1", "scene1", editableActor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-editable");
    const nonEditableToken = buildTokenFromActor("w1", "scene1", nonEditableActor, "link", { x: 100, y: 0 }, { w: 100, h: 100 }, "tok-non-editable");
    const store = storeWith(
      buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"),
      editableActor,
      nonEditableActor,
      editableToken,
      nonEditableToken,
    );
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok-editable", "tok-non-editable"]);
    render(ConditionsPanel, {
      context: setAppContextForTest({
        role: "player",
        world: "w1",
        documents: store,
        dispatchIntent,
        tokenSelection,
        canEdit: (doc) => doc.id === "act-editable",
      }),
    });

    // The palette chip reads active (matches the editable subset's true state), not mixed —
    // the non-editable token, which lacks the condition, no longer governs the display.
    const chip = screen.getByTitle("Dead");
    expect(chip.getAttribute("aria-pressed")).toBe("true");

    // Clicking correctly computes REMOVE for the editable target: `toggle` derives direction
    // from `isActive`'s verdict over the SAME canEdit-gated set it mutates, so the direction
    // and the target always agree.
    await fireEvent.click(chip);
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0]).toMatchObject({ op: "update", doc_id: "act-editable", changes: [{ path: "/engine/conditions", new: [] }] });
  });

  it("isActive reports inactive when the selection has only non-editable targets", async () => {
    const dispatchIntent = vi.fn();
    const nonEditableActor = actorDoc("act-non-editable", ["dead"]);
    const nonEditableToken = buildTokenFromActor("w1", "scene1", nonEditableActor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok-non-editable");
    const store = storeWith(
      buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1"),
      nonEditableActor,
      nonEditableToken,
    );
    const tokenSelection = new TokenSelection();
    tokenSelection.set(["tok-non-editable"]);
    render(ConditionsPanel, {
      context: setAppContextForTest({
        role: "player",
        world: "w1",
        documents: store,
        dispatchIntent,
        tokenSelection,
        canEdit: () => false,
      }),
    });

    // Zero editable targets: matches isActive's existing "no targets → false" convention rather
    // than reflecting a token the user cannot affect as "active".
    const chip = screen.getByTitle("Dead");
    expect(chip.getAttribute("aria-pressed")).toBe("false");

    await fireEvent.click(chip);
    expect(dispatchIntent).not.toHaveBeenCalled();
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

