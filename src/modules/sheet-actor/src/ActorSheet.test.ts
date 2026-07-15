import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { ContributionRegistry, sheetContract, DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ActorSheet from "./ActorSheet.svelte";
import { sheetActor } from "./index";

function storeWith(system: unknown) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: envelope("w1", "actor", null, system, "a1") }] });
  return s;
}

describe("module-sheet-actor registration", () => {
  it("registers ActorSheet under shadowcat.sheet:actor at priority 0", () => {
    const contributions = new ContributionRegistry();
    // Mirrors the real ModuleContext.contributions wrapper (modules.ts activate): a
    // 1-arg contribute(c) closure that auto-injects the module id — index.ts never
    // self-declares it, matching every other first-party module.
    sheetActor.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-actor" }) },
    } as never);
    const entry = contributions.entriesFor(sheetContract("actor"))[0];
    expect(entry?.contribution.sheet?.priority).toBe(0);
    expect(entry?.module).toBe("sheet-actor");
  });
});

describe("ActorSheet edits", () => {
  it("edits the name with the real pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "a1", changes: [{ path: "/system/name", old: "Goblin", new: "Orc" }] }]]);
  });

  it("disables controls for a non-editor (canEdit false)", () => {
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, canEdit: () => false, role: "player" });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByLabelText("sheetActor.name") as HTMLInputElement).disabled).toBe(true);
  });

  it("editing sizeW dispatches the real current {w,h} pair as old, preserving h in new", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 2, h: 3 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetActor.sizeW"), { target: { value: "5" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "a1", changes: [{ path: "/system/size", old: { w: 2, h: 3 }, new: { w: 5, h: 3 } }] }]]);
  });

  // Regression test for the reactive-subscription fix: a second edit in the same rendered
  // instance must read the FIRST edit's result as `old`, not the doc snapshot frozen at first
  // render. Fails against the pre-fix frozen-`ctx.documents.get()` derived because the second
  // dispatch's `old` would still be the pre-edit doc's field, not the post-edit one.
  it("a second edit in the same instance dispatches a fresh old reflecting the first edit", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const dispatchIntent = (ops: unknown) => {
      calls.push(ops);
      // Simulate the server confirming the intent: apply it back onto the store as an
      // authoritative command so the reactive subscribers (subscribe()-calling deriveds) re-run.
      const changes = (ops as { changes: { path: string; new: unknown }[] }[])[0].changes;
      documents.applyCommand({
        seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
        ops: [{ op: "update", doc_id: "a1", changes }],
      });
    };
    const context = setAppContextForTest({ documents, dispatchIntent, canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc" } });
    await fireEvent.change(getByLabelText("sheetActor.name"), { target: { value: "Orc Warlord" } });

    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/system/name", old: "Goblin", new: "Orc" }] }],
      [{ op: "update", doc_id: "a1", changes: [{ path: "/system/name", old: "Orc", new: "Orc Warlord" }] }],
    ]);
  });

  // Combines the compound-field and repeat-edit scenarios: editing sizeH after sizeW must
  // read the FIRST edit's w back as old/new, not the pre-render snapshot. Fails against the
  // pre-fix frozen doc because the second dispatch would revert w to its original value.
  it("editing sizeH after sizeW preserves the first edit's w, not the frozen pre-render value", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Goblin", displayName: "Creature", faction: null, shape: "square", size: { w: 1, h: 1 }, conditions: [], prototype: false, visual: { kind: "image", asset: "x" } });
    const dispatchIntent = (ops: unknown) => {
      calls.push(ops);
      const changes = (ops as { changes: { path: string; new: unknown }[] }[])[0].changes;
      documents.applyCommand({
        seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
        ops: [{ op: "update", doc_id: "a1", changes }],
      });
    };
    const context = setAppContextForTest({ documents, dispatchIntent, canEdit: () => true });
    const { getByLabelText } = render(ActorSheet, { props: { docId: "a1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.change(getByLabelText("sheetActor.sizeW"), { target: { value: "5" } });
    await fireEvent.change(getByLabelText("sheetActor.sizeH"), { target: { value: "9" } });

    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/system/size", old: { w: 1, h: 1 }, new: { w: 5, h: 1 } }] }],
      [{ op: "update", doc_id: "a1", changes: [{ path: "/system/size", old: { w: 5, h: 1 }, new: { w: 5, h: 9 } }] }],
    ]);
  });
});
