import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { ContributionRegistry, sheetContract, DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ItemSheet from "./ItemSheet.svelte";
import { sheetItem } from "./index";

function storeWith(system: unknown) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: envelope("w1", "item", null, system, "i1") }] });
  return s;
}

describe("module-sheet-item registration", () => {
  it("registers ItemSheet under shadowcat.sheet:item at priority 0", () => {
    const contributions = new ContributionRegistry();
    // Mirrors the real ModuleContext.contributions wrapper, which threads the module id through
    // to `contribute` as a second argument.
    sheetItem.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-item" }) },
    } as never);
    const entry = contributions.entriesFor(sheetContract("item"))[0];
    expect(entry?.contribution.sheet?.priority).toBe(0);
    expect(entry?.module).toBe("sheet-item");
  });
});

describe("ItemSheet dice roll-to-chat", () => {
  it("posts /roll to chat for a dice-notation field value", async () => {
    const sent: unknown[] = [];
    const documents = storeWith({ name: "Sword", damage: "1d8+2" });
    const context = setAppContextForTest({ documents, chat: { send: (o) => sent.push(o), edit: () => {}, delete: () => {} }, canEdit: () => true });
    const { getByRole } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByRole("button", { name: "sheetItem.roll" }));
    expect(sent).toEqual([{ channel: "general", content: "/roll 1d8+2" }]);
  });

  it("edits the item name with the real pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Sword" });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetItem.name"), { target: { value: "Axe" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "i1", changes: [{ path: "/system/name", old: "Sword", new: "Axe" }] }]]);
  });
});
