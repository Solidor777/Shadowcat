import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, buildTableDoc, buildChannelRegistryDoc, type WireDocument, type WireSearchHit } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import TablesPanel from "./TablesPanel.svelte";

const SELF = "u-self";

function engine() {
  return { draw: { kind: "weighted" as const }, rows: [], description: "" };
}

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

describe("TablesPanel list", () => {
  it("lists every table, sorted by name", () => {
    const b = buildTableDoc("w1", "Zed Table", engine(), { id: "t2" });
    const a = buildTableDoc("w1", "Ace Table", engine(), { id: "t1" });
    const documents = storeWith(b, a);
    const { getAllByTestId } = render(TablesPanel, { context: setAppContextForTest({ documents }) });
    const rows = getAllByTestId("table-row");
    expect(rows.map((r) => r.dataset.tableId)).toEqual(["t1", "t2"]);
  });

  it("shows the empty message when there are no tables", () => {
    const { getByText } = render(TablesPanel, { context: setAppContextForTest({ documents: storeWith() }) });
    expect(getByText("tables.empty")).toBeTruthy();
  });
});

describe("TablesPanel search", () => {
  it("sends docTypes: [\"table\"] and replaces the list with the hit list", async () => {
    const listed = buildTableDoc("w1", "Listed", engine(), { id: "t1" });
    const hitDoc = buildTableDoc("w1", "Loot", engine(), { id: "t2" });
    const documents = storeWith(listed);
    const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const { getByTestId, getAllByTestId } = render(TablesPanel, { context: setAppContextForTest({ documents, searchDocuments }) });
    await fireEvent.input(getByTestId("tables-search"), { target: { value: "loot" } });
    expect(searchDocuments).toHaveBeenCalledWith(
      "loot",
      expect.objectContaining({ docTypes: ["table"] }),
      expect.any(Function),
    );
    const rows = getAllByTestId("table-row");
    expect(rows.map((r) => r.dataset.tableId)).toEqual(["t2"]);
  });
});

describe("TablesPanel create", () => {
  it("builds a table with owner: selfId reaching grantAuthor, and opens it", async () => {
    const calls: unknown[] = [];
    const opened: unknown[] = [];
    const context = setAppContextForTest({
      documents: storeWith(),
      canCreate: () => true,
      selfId: SELF,
      world: "w1",
      dispatchIntent: (ops) => calls.push(ops),
      openDocument: (ref) => opened.push(ref),
    });
    const { getByTestId } = render(TablesPanel, { context });
    await fireEvent.input(getByTestId("tables-name"), { target: { value: "Loot" } });
    await fireEvent.click(getByTestId("tables-create"));
    expect(calls).toHaveLength(1);
    const created = (calls[0] as [{ op: string; doc: WireDocument }])[0];
    expect(created.op).toBe("create");
    expect(created.doc.name).toBe("Loot");
    expect(created.doc.permissions.capabilities.by_role.owner).toEqual(
      expect.arrayContaining(["core:delete", "core:edit_permissions"]),
    );
    expect(opened).toEqual([{ docId: created.doc.id }]);
  });

  it("hides the create form when canCreate is false", () => {
    const { queryByTestId } = render(TablesPanel, { context: setAppContextForTest({ documents: storeWith(), canCreate: () => false }) });
    expect(queryByTestId("tables-name")).toBeNull();
  });
});

describe("TablesPanel row gating and draw", () => {
  it("shows Delete when the fixture's canDelete returns true, never keyed on raw owner", () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" }); // no owner stamped
    const shown = render(TablesPanel, { context: setAppContextForTest({ documents: storeWith(doc), role: "player", canDelete: () => true }) });
    expect(shown.getByTestId("table-delete")).toBeTruthy();
  });

  it("hides Delete when the fixture's canDelete returns false", () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const hidden = render(TablesPanel, { context: setAppContextForTest({ documents: storeWith(doc), role: "player", canDelete: () => false }) });
    expect(hidden.queryByTestId("table-delete")).toBeNull();
  });

  it("quick-draw is disabled with no channel", () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const noChannel = render(TablesPanel, { context: setAppContextForTest({ documents: storeWith(doc) }) });
    expect((noChannel.getByTestId("table-quick-draw") as HTMLButtonElement).disabled).toBe(true);
  });

  it("quick-draw calls drawTable with the channel and count once one exists", async () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const drawTable = vi.fn(() => Promise.resolve());
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const withChannel = render(TablesPanel, { context: setAppContextForTest({ documents, chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), drawTable } }) });
    expect((withChannel.getByTestId("table-quick-draw") as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.click(withChannel.getByTestId("table-quick-draw"));
    expect(drawTable).toHaveBeenCalledWith({ tableId: "t1", channel: "general", count: 1 });
  });

  it("a rejected draw notifies with the reason", async () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const notified: string[] = [];
    const drawTable = vi.fn(() => Promise.reject(new Error("EmptyTable")));
    const { getByTestId } = render(TablesPanel, {
      context: setAppContextForTest({
        documents,
        notify: (m) => notified.push(m),
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), drawTable },
      }),
    });
    await fireEvent.click(getByTestId("table-quick-draw"));
    await Promise.resolve();
    await Promise.resolve();
    expect(notified).toEqual(["EmptyTable"]);
  });
});
