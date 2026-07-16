import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { ContributionRegistry, sheetContract, DocumentStore, envelope } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ItemSheet from "./ItemSheet.svelte";
import { sheetItem } from "./index";

/** Builds an item doc on the three-band shape: `name` is pulled out onto the envelope
 * (matching `buildItemDoc`'s contract), the rest stays in the opaque `system` tree. */
function storeWith(fields: Record<string, unknown>) {
  const { name, ...system } = fields;
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: [{ op: "create", doc: envelope("w1", "item", null, system, "i1", undefined, (name as string | null) ?? null) }],
  });
  return s;
}

/** Builds a PARENT actor doc ("a1") with a single embedded item at `/embedded/item/0`
 * (mirrors `resolveDocRef`'s embedded-child resolution: `docId` resolves to the parent,
 * `systemPrefix` to `/embedded/item/0/system`). Also seeds a top-level item ("i1") with a
 * DIFFERENT name so a bug that reads/writes the parent's own `/name` band is distinguishable
 * from a bug that reads/writes the wrong item. */
function storeWithEmbeddedItem(actorName: string, itemFields: Record<string, unknown>) {
  const { name: itemName, ...itemSystem } = itemFields;
  const s = new DocumentStore();
  const actor = envelope("w1", "actor", null, {}, "a1", { displayName: actorName }, actorName);
  actor.embedded = { item: [envelope("w1", "item", null, itemSystem, "embedded-i0", undefined, (itemName as string | null) ?? null)] };
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: actor }] });
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
    // Roll buttons carry no aria-label (their visible text "key: formula" is a distinct,
    // per-item-unique accessible name, unlike a shared static label) — locate by that text.
    const { getByRole } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByRole("button", { name: "damage: 1d8+2" }));
    expect(sent).toEqual([{ channel: "general", content: "/roll 1d8+2" }]);
  });

  it("edits the item name with the real pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Sword" });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByLabelText("sheetItem.name"), { target: { value: "Axe" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "i1", changes: [{ path: "/name", old: "Sword", new: "Axe" }] }]]);
  });

  // Regression test for the reactive-subscription fix: a second edit in the same rendered
  // instance must read the FIRST edit's result as `old`, not the doc snapshot frozen at first
  // render. Mirrors ActorSheet.test.ts's "a second edit in the same instance dispatches a
  // fresh old reflecting the first edit". Fails against the pre-fix frozen-`ctx.documents.get()`
  // derived because the second dispatch's `old` would still be the pre-edit doc's field.
  it("a second edit in the same instance dispatches a fresh old reflecting the first edit", async () => {
    const calls: unknown[] = [];
    const documents = storeWith({ name: "Sword" });
    const dispatchIntent = (ops: unknown) => {
      calls.push(ops);
      // Simulate the server confirming the intent: apply it back onto the store as an
      // authoritative command so the reactive subscribers (subscribe()-calling deriveds) re-run.
      const changes = (ops as { changes: { path: string; new: unknown }[] }[])[0].changes;
      documents.applyCommand({
        seq: documents.appliedSeq + 1, world_id: "w1", author: "u", ts: 0,
        ops: [{ op: "update", doc_id: "i1", changes }],
      });
    };
    const context = setAppContextForTest({ documents, dispatchIntent, canEdit: () => true });
    const { getByLabelText } = render(ItemSheet, { props: { docId: "i1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.change(getByLabelText("sheetItem.name"), { target: { value: "Axe" } });
    await fireEvent.change(getByLabelText("sheetItem.name"), { target: { value: "Battle Axe" } });

    expect(calls).toEqual([
      [{ op: "update", doc_id: "i1", changes: [{ path: "/name", old: "Sword", new: "Axe" }] }],
      [{ op: "update", doc_id: "i1", changes: [{ path: "/name", old: "Axe", new: "Battle Axe" }] }],
    ]);
  });

  // For an item opened from an actor's inventory, `docId` resolves to the PARENT actor's
  // document id and `systemPrefix` to `/embedded/item/0/system`. Reading `doc?.name` directly
  // (the parent actor's own envelope name) or writing the literal `/name` path renames the
  // PARENT ACTOR instead of the item. Asserts both the displayed name comes from the embedded
  // item's own node and an edit targets `/embedded/item/0/name`, never `/name`.
  it("reads and writes the embedded item's own name, not the parent actor's", async () => {
    const calls: unknown[] = [];
    const documents = storeWithEmbeddedItem("Aria the Wizard", { name: "Wand of Fire" });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByLabelText } = render(ItemSheet, {
      props: { docId: "a1", systemPrefix: "/embedded/item/0/system", close: () => {} },
      context,
    });

    // (a) displayed name is the embedded item's, not the parent actor's.
    const input = getByLabelText("sheetItem.name") as HTMLInputElement;
    expect(input.value).toBe("Wand of Fire");

    // (b) an edit writes /embedded/item/0/name, never the literal /name (which would rename
    // the parent actor "Aria the Wizard").
    await fireEvent.change(input, { target: { value: "Staff of Fire" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "a1", changes: [{ path: "/embedded/item/0/name", old: "Wand of Fire", new: "Staff of Fire" }] }],
    ]);
  });
});
