import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, buildTableDoc, buildTableFolderDoc, buildChannelRegistryDoc, type WireDocument, type WireSearchHit } from "@shadowcat/core";
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

describe("TablesPanel context menu", () => {
  it("right-click offers Open, Draw and Delete, and each runs the handler its button runs", async () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const drawTable = vi.fn(() => Promise.resolve());
    const opened: unknown[] = [];
    const dispatched: unknown[] = [];
    const { getByTestId, queryByTestId } = render(TablesPanel, {
      context: setAppContextForTest({
        documents,
        canDelete: () => true,
        openDocument: (ref) => opened.push(ref),
        dispatchIntent: (ops) => dispatched.push(ops),
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), reviseAudience: () => Promise.resolve(), drawTable },
      }),
    });

    await fireEvent.contextMenu(getByTestId("table-row"));
    await fireEvent.click(getByTestId("context-menu-item-open"));
    expect(queryByTestId("context-menu")).toBeNull();
    await fireEvent.contextMenu(getByTestId("table-row"));
    await fireEvent.click(getByTestId("context-menu-item-draw"));
    await fireEvent.contextMenu(getByTestId("table-row"));
    await fireEvent.click(getByTestId("context-menu-item-delete"));

    await fireEvent.click(getByTestId("table-open"));
    await fireEvent.click(getByTestId("table-quick-draw"));
    await fireEvent.click(getByTestId("table-delete"));
    expect(opened).toEqual([{ docId: "t1" }, { docId: "t1" }]);
    expect(drawTable.mock.calls).toEqual([
      [{ tableId: "t1", channel: "general", count: 1 }],
      [{ tableId: "t1", channel: "general", count: 1 }],
    ]);
    expect(dispatched).toEqual([[{ op: "delete", doc }], [{ op: "delete", doc }]]);
  });

  it("the menu gates Delete on canDelete and disables Draw with no channel, as the buttons do", async () => {
    const doc = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const { getByTestId, queryByTestId } = render(TablesPanel, {
      context: setAppContextForTest({ documents: storeWith(doc), canDelete: () => false }),
    });

    await fireEvent.contextMenu(getByTestId("table-row"));

    expect(queryByTestId("context-menu-item-delete")).toBeNull();
    expect((getByTestId("context-menu-item-draw") as HTMLButtonElement).disabled).toBe(true);
  });

  it("the menu opens for the row that was clicked, not the first row", async () => {
    const a = buildTableDoc("w1", "A", engine(), { id: "t1" });
    const b = buildTableDoc("w1", "B", engine(), { id: "t2" });
    const opened: unknown[] = [];
    const { getAllByTestId, getByTestId } = render(TablesPanel, {
      context: setAppContextForTest({ documents: storeWith(a, b), openDocument: (ref) => opened.push(ref) }),
    });

    await fireEvent.contextMenu(getAllByTestId("table-row")[1]);
    await fireEvent.click(getByTestId("context-menu-item-open"));

    expect(opened).toEqual([{ docId: "t2" }]);
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
    const withChannel = render(TablesPanel, { context: setAppContextForTest({ documents, chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), reviseAudience: () => Promise.resolve(), drawTable } }) });
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
        chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve(), recalc: () => Promise.resolve(), reviseAudience: () => Promise.resolve(), drawTable },
      }),
    });
    await fireEvent.click(getByTestId("table-quick-draw"));
    await Promise.resolve();
    await Promise.resolve();
    expect(notified).toEqual(["EmptyTable"]);
  });
});

describe("TablesPanel folders", () => {
  /** A 40px-tall rect at `top`. */
  const rect = (top: number): DOMRect => ({ top, height: 40, bottom: top + 40, left: 0, right: 100, width: 100, x: 0, y: top, toJSON: () => ({}) });

  /** Folders `fa` (root) and `fb`, tables `t1` in `fa`, `t2` at the root. */
  function foldered() {
    return storeWith(
      buildTableFolderDoc("w1", "Alpha", { id: "fa" }),
      buildTableFolderDoc("w1", "Beta", { id: "fb" }),
      buildTableDoc("w1", "In Alpha", engine(), { id: "t1", parentId: "fa" }),
      buildTableDoc("w1", "At Root", engine(), { id: "t2" }),
    );
  }

  /** Renders the panel over `foldered()`, folder-row and all-row rects stubbed 40px apart:
   * all 0..40, fa 40..80, fb 80..120. */
  function mount(over: Record<string, unknown> = {}) {
    const dispatched: unknown[][] = [];
    const context = setAppContextForTest({
      documents: foldered(),
      world: "w1",
      selfId: SELF,
      canCreate: () => true,
      canDelete: () => true,
      canEdit: () => true,
      dispatchIntent: (ops: unknown[]) => dispatched.push(ops),
      ...over,
    } as never);
    const result = render(TablesPanel, { context });
    const stub = (el: HTMLElement | null, top: number) => {
      if (el) el.getBoundingClientRect = () => rect(top);
    };
    stub(result.getByTestId("folder-node-all"), 0);
    stub(result.getByTestId("folder-node-fa").closest(".row"), 40);
    stub(result.getByTestId("folder-node-fb").closest(".row"), 80);
    return { ...result, dispatched };
  }

  /** Drags table `id`'s handle and releases at `y`. */
  async function dragTable(id: string, y: number): Promise<void> {
    const handle = document.querySelector(`[data-testid="table-drag"][data-table-id="${id}"]`) as HTMLElement;
    await fireEvent.pointerDown(handle, { clientY: 200 });
    await fireEvent(window, new PointerEvent("pointerup", { clientX: 10, clientY: y }));
  }

  it("lists every table under All and only the selected folder's tables under a folder", async () => {
    const { getAllByTestId, getByTestId } = mount();
    expect(getAllByTestId("table-row").map((r) => r.dataset.tableId)).toEqual(["t2", "t1"]);
    await fireEvent.click(getByTestId("folder-node-fa"));
    expect(getAllByTestId("table-row").map((r) => r.dataset.tableId)).toEqual(["t1"]);
    await fireEvent.click(getByTestId("folder-node-all"));
    expect(getAllByTestId("table-row")).toHaveLength(2);
  });

  it("a table created while a folder is selected is filed under it", async () => {
    const { getByTestId, dispatched } = mount();
    await fireEvent.click(getByTestId("folder-node-fb"));
    await fireEvent.input(getByTestId("tables-name"), { target: { value: "Loot" } });
    await fireEvent.click(getByTestId("tables-create"));
    const created = (dispatched[0] as [{ doc: WireDocument }])[0].doc;
    expect(created.doc_type).toBe("table");
    expect(created.parent_id).toBe("fb");
  });

  it("a folder created in the panel is a table_folder owned by the creator", async () => {
    const { getByTestId, dispatched } = mount();
    const input = getByTestId("folder-create-name");
    await fireEvent.input(input, { target: { value: "Gamma" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    const doc = (dispatched[0] as [{ doc: WireDocument }])[0].doc;
    expect(doc.doc_type).toBe("table_folder");
    expect(doc.name).toBe("Gamma");
    expect(doc.permissions.users).toEqual({ [SELF]: "owner" });
  });

  it("dragging a table onto a folder row dispatches a Move with the stored parent as the pre-image", async () => {
    const { dispatched } = mount();
    await dragTable("t2", 60); // middle of fa (40..80)
    expect(dispatched).toEqual([[{ op: "move", doc_id: "t2", parent_id: "fa", old_parent_id: null }]]);
  });

  it("dragging a table onto the All row moves it to the root", async () => {
    const { dispatched } = mount();
    await dragTable("t1", 20); // middle of the all row (0..40)
    expect(dispatched).toEqual([[{ op: "move", doc_id: "t1", parent_id: null, old_parent_id: "fa" }]]);
  });

  it("dropping a table on its own folder dispatches nothing", async () => {
    const { dispatched } = mount();
    await dragTable("t1", 60);
    expect(dispatched).toEqual([]);
  });

  it("the Move-to picker and the context menu's Move item dispatch the same Move", async () => {
    const { getByTestId, getAllByTestId, dispatched } = mount();
    await fireEvent.click(getAllByTestId("table-move")[0]); // t2 (sorted first)
    await fireEvent.click(getByTestId("table-move-target-fb"));
    await fireEvent.contextMenu(getAllByTestId("table-row")[0]);
    await fireEvent.click(getByTestId("context-menu-item-move"));
    await fireEvent.click(getByTestId("table-move-target-fb"));
    const move = [{ op: "move", doc_id: "t2", parent_id: "fb", old_parent_id: null }];
    expect(dispatched).toEqual([move, move]);
  });

  it("a viewer who cannot change a table's parent gets no drag handle or move button", () => {
    const { queryByTestId } = mount({ canEdit: () => false });
    expect(queryByTestId("table-drag")).toBeNull();
    expect(queryByTestId("table-move")).toBeNull();
  });

  it("deleting a folder dispatches a plain Delete of the folder document", async () => {
    const { getByTestId, dispatched } = mount();
    await fireEvent.click(getByTestId("folder-delete-fb"));
    await fireEvent.click(getByTestId("folder-delete-confirm"));
    const op = (dispatched[0] as [{ op: string; doc: WireDocument }])[0];
    expect(op.op).toBe("delete");
    expect(op.doc.id).toBe("fb");
  });

  it("a search spans every folder", async () => {
    const hit = buildTableDoc("w1", "In Alpha", engine(), { id: "t1", parentId: "fa" });
    const searchDocuments = vi.fn((_q: string, _o: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hit, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const { getByTestId, getAllByTestId } = mount({ searchDocuments });
    await fireEvent.click(getByTestId("folder-node-fb"));
    await fireEvent.input(getByTestId("tables-search"), { target: { value: "alpha" } });
    expect(getAllByTestId("table-row").map((r) => r.dataset.tableId)).toEqual(["t1"]);
  });

  it("a user who cannot create folders sees no folder mutation controls", () => {
    const { queryByTestId } = mount({ canCreate: () => false });
    expect(queryByTestId("folder-create-name")).toBeNull();
    expect(queryByTestId("folder-delete-fa")).toBeNull();
  });
});
