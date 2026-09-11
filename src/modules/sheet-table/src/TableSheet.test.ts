import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, buildTableDoc, buildChannelRegistryDoc, type WireDocument, type TableEngine, type WireOperation, type WireSearchHit } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import TableSheet from "./TableSheet.svelte";

function engine(rows: TableEngine["rows"] = []): TableEngine {
  return { draw: { kind: "weighted" }, rows, description: "" };
}

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

/** A `dispatchIntent` stand-in that records every call AND applies it back onto `documents`
 * as an authoritative command, simulating the server confirming the write — so a test that
 * asserts on the sheet's OWN re-render after a change (e.g. a revealed field) observes it,
 * mirroring `ItemSheet.test.ts`'s "a second edit ... reflecting the first edit" pattern. */
function confirmingDispatch(documents: DocumentStore, calls: unknown[]) {
  let seq = 1;
  return (ops: WireOperation[]) => {
    calls.push(ops);
    const updates = ops.filter((op): op is Extract<WireOperation, { op: "update" }> => op.op === "update");
    if (updates.length === 0) return;
    documents.applyCommand({
      seq: ++seq, world_id: "w1", author: "u", ts: 0,
      ops: updates,
    });
  };
}

describe("TableSheet header", () => {
  it("edits the name with the real pre-image", async () => {
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("table-name"), { target: { value: "Better Loot" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "t1", changes: [{ path: "/name", old: "Loot", new: "Better Loot" }] }]]);
  });

  it("edits the description with the real pre-image", async () => {
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("table-description"), { target: { value: "New description" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "t1", changes: [{ path: "/engine/description", old: "", new: "New description" }] }]]);
  });

  it("switching the draw rule to formula writes the WHOLE draw object plus the reshaped rows, atomically, and reveals the notation input", async () => {
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: confirmingDispatch(documents, calls), canEdit: () => true });
    const { getByTestId, queryByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("table-notation")).toBeNull();
    await fireEvent.change(getByTestId("table-draw-rule"), { target: { value: "formula" } });
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [
        { path: "/engine/draw", old: { kind: "weighted" }, new: { kind: "formula", notation: "1d20" } },
        { path: "/engine/rows", old: [], new: [] },
      ],
    }]]);
    expect(getByTestId("table-notation")).toBeTruthy();
  });

  it("switching weighted to formula with existing rows dispatches ONE Update reshaping every row's range with pairwise-disjoint ranges", async () => {
    const weightedRows = [
      { weight: 1, range: null, label: "a", results: [] },
      { weight: 2, range: null, label: "b", results: [] },
    ];
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine(weightedRows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("table-draw-rule"), { target: { value: "formula" } });
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [
        { path: "/engine/draw", old: { kind: "weighted" }, new: { kind: "formula", notation: "1d20" } },
        { path: "/engine/rows", old: weightedRows, new: expect.any(Array) },
      ],
    }]]);
    const dispatched = (calls[0] as { changes: { new: unknown }[] }[])[0].changes[1].new as {
      weight: number;
      range: { lo: number; hi: number } | null;
      label: string;
      results: unknown[];
    }[];
    expect(dispatched.map((r) => ({ weight: r.weight, label: r.label, results: r.results }))).toEqual([
      { weight: 1, label: "a", results: [] },
      { weight: 2, label: "b", results: [] },
    ]);
    const ranges = dispatched.map((r) => r.range);
    expect(ranges.every((r) => r !== null && r.lo <= r.hi)).toBe(true);
    for (let i = 0; i < ranges.length; i++) {
      for (let j = i + 1; j < ranges.length; j++) {
        const a = ranges[i]!;
        const b = ranges[j]!;
        expect(a.lo <= b.hi && b.lo <= a.hi).toBe(false);
      }
    }
  });

  it("switching formula to weighted with existing rows dispatches ONE Update clearing every row's range", async () => {
    const formulaRows = [
      { weight: 1, range: { lo: 1, hi: 10 }, label: "a", results: [] },
      { weight: 2, range: { lo: 11, hi: 20 }, label: "b", results: [] },
    ];
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", { draw: { kind: "formula" as const, notation: "1d20" }, rows: formulaRows, description: "" }, { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("table-draw-rule"), { target: { value: "weighted" } });
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [
        { path: "/engine/draw", old: { kind: "formula", notation: "1d20" }, new: { kind: "weighted" } },
        {
          path: "/engine/rows", old: formulaRows,
          new: [
            { weight: 1, range: null, label: "a", results: [] },
            { weight: 2, range: null, label: "b", results: [] },
          ],
        },
      ],
    }]]);
  });

  it("draws to chat over the resolved channel with the count input's value", async () => {
    const drawTable = vi.fn().mockResolvedValue(undefined);
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents, canEdit: () => true,
      chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), drawTable },
    });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("table-draw-count"), { target: { value: "3" } });
    await fireEvent.click(getByTestId("table-draw"));
    expect(drawTable).toHaveBeenCalledWith({ tableId: "t1", channel: "general", count: 3 });
  });

  it("disables Draw and shows the no-channel notice when no channel-registry doc exists", () => {
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc); // no channel-registry
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId, getByText } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByTestId("table-draw") as HTMLButtonElement).disabled).toBe(true);
    expect(getByText("sheetTable.noChannel")).toBeTruthy();
  });

  it("surfaces the server's draw-rejection reason via ctx.notify", async () => {
    const drawTable = vi.fn().mockRejectedValue(new Error("Table has no rows"));
    const notify = vi.fn();
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents, canEdit: () => true, notify,
      chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), drawTable },
    });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("table-draw"));
    await vi.waitFor(() => expect(notify).toHaveBeenCalledWith("Table has no rows"));
  });

  it("disables every write control for a non-editor (canEdit false)", () => {
    const doc = buildTableDoc("w1", "Loot", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => false });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    expect((getByTestId("table-name") as HTMLInputElement).disabled).toBe(true);
    expect((getByTestId("table-add-row") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("TableSheet rows", () => {
  it("adding a row writes the WHOLE rows array with the raw stored array as old", async () => {
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine([{ weight: 1, range: null, label: "a", results: [] }]), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("table-add-row"));
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{
        path: "/engine/rows",
        old: [{ weight: 1, range: null, label: "a", results: [] }],
        new: [{ weight: 1, range: null, label: "a", results: [] }, { weight: 1, range: null, label: "", results: [] }],
      }],
    }]]);
  });

  it("editing a row's label writes the whole rows array with the edited row replaced", async () => {
    const calls: unknown[] = [];
    const doc = buildTableDoc("w1", "Loot", engine([{ weight: 1, range: null, label: "a", results: [] }]), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("row-label"), { target: { value: "b" } });
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: [{ weight: 1, range: null, label: "a", results: [] }], new: [{ weight: 1, range: null, label: "b", results: [] }] }],
    }]]);
  });

  it("removing a row writes the shortened array", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [] }, { weight: 1, range: null, label: "b", results: [] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getAllByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getAllByTestId("row-remove")[0]);
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: rows, new: [rows[1]] }],
    }]]);
  });

  it("moving a row down swaps its position; the first row's up button is disabled", () => {
    const rows = [{ weight: 1, range: null, label: "a", results: [] }, { weight: 1, range: null, label: "b", results: [] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getAllByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    expect((getAllByTestId("row-up")[0] as HTMLButtonElement).disabled).toBe(true);
    expect((getAllByTestId("row-down")[1] as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders lo/hi range inputs only under a formula draw rule", () => {
    const rows = [{ weight: 1, range: { lo: 1, hi: 10 }, label: "a", results: [] }];
    const doc = buildTableDoc("w1", "Loot", { draw: { kind: "formula", notation: "1d20" }, rows, description: "" }, { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByTestId("row-lo")).toBeTruthy();
    expect(getByTestId("row-hi")).toBeTruthy();
  });
});

describe("TableSheet entries", () => {
  it("adding an entry appends a default text entry to the row, via the whole-rows write", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("row-add-entry"));
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: rows, new: [{ ...rows[0], results: [{ kind: "text", text: "" }] }] }],
    }]]);
  });

  it("editing a text entry's content propagates through the row into the whole-rows write", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "text" as const, text: "" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("entry-text"), { target: { value: "You find a key." } });
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: rows, new: [{ ...rows[0], results: [{ kind: "text", text: "You find a key." }] }] }],
    }]]);
  });

  it("removing an entry shortens the row's results", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "text" as const, text: "x" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("entry-remove"));
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: rows, new: [{ ...rows[0], results: [] }] }],
    }]]);
  });

  it("changing entry kind to doc reveals the label input and doc picker", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "text" as const, text: "x" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: confirmingDispatch(documents, calls), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("entry-kind"), { target: { value: "doc" } });
    expect(getByTestId("entry-label")).toBeTruthy();
    expect(getByTestId("entry-pick-doc")).toBeTruthy();
  });

  it("changing entry kind to image reveals the pick-image button and alt input", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "text" as const, text: "x" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: confirmingDispatch(documents, calls), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("entry-kind"), { target: { value: "image" } });
    expect(getByTestId("entry-pick-image")).toBeTruthy();
    expect(getByTestId("entry-alt")).toBeTruthy();
  });

  it("changing entry kind to draw reveals the table picker and count input", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "text" as const, text: "x" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: confirmingDispatch(documents, calls), canEdit: () => true });
    const { getByTestId } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("entry-kind"), { target: { value: "draw" } });
    expect(getByTestId("entry-pick-table")).toBeTruthy();
    expect(getByTestId("entry-count")).toBeTruthy();
  });

  it("the doc picker's live search sends the query and picking sets target/label", async () => {
    const calls: unknown[] = [];
    const rows = [{ weight: 1, range: null, label: "a", results: [{ kind: "doc" as const, target: { kind: "doc" as const, doc_id: "", embedded_path: null }, label: "" }] }];
    const doc = buildTableDoc("w1", "Loot", engine(rows), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const hitDoc = buildTableDoc("w1", "Rusty Key Table", engine(), { id: "hit1" });
    const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true, searchDocuments });
    const { getByTestId, getByText } = render(TableSheet, { props: { docId: "t1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.input(getByTestId("entry-pick-doc"), { target: { value: "rusty" } });
    expect(searchDocuments).toHaveBeenCalledWith("rusty", { limit: 20 }, expect.any(Function));
    await fireEvent.click(getByText("Rusty Key Table"));
    expect(calls).toEqual([[{
      op: "update", doc_id: "t1",
      changes: [{ path: "/engine/rows", old: rows, new: [{ ...rows[0], results: [{ kind: "doc", target: { kind: "doc", doc_id: "hit1", embedded_path: null }, label: "Rusty Key Table" }] }] }],
    }]]);
  });
});
