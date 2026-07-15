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
});
