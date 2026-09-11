import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, buildNoteDoc, type WireDocument, type WireSearchHit } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NotesPanel from "./NotesPanel.svelte";

const SELF = "u-self";

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

describe("NotesPanel tree", () => {
  it("renders every note the caller can see, promoting a hidden-parent child to root", () => {
    const root = buildNoteDoc("w1", "Root", "", { id: "r1", owner: SELF });
    const orphan = buildNoteDoc("w1", "Orphan", "", { id: "o1", parentId: "unreadable-parent" });
    const documents = storeWith(root, orphan);
    const context = setAppContextForTest({ documents, role: "gm" });
    const { getAllByTestId } = render(NotesPanel, { context });
    const rows = getAllByTestId("note-row");
    expect(rows.map((r) => r.dataset.noteId)).toEqual(expect.arrayContaining(["r1", "o1"]));
  });

  it("shows the empty message when there are no notes", () => {
    const context = setAppContextForTest({ documents: storeWith() });
    const { getByText } = render(NotesPanel, { context });
    expect(getByText("notes.empty")).toBeTruthy();
  });
});

describe("NotesPanel search", () => {
  it("sends docTypes: [\"note\"] and replaces the tree with the hit list", async () => {
    const treeNote = buildNoteDoc("w1", "Tree note", "", { id: "n1", owner: SELF });
    const hitDoc = buildNoteDoc("w1", "Found note", "", { id: "n2", owner: SELF });
    const documents = storeWith(treeNote);
    const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const context = setAppContextForTest({ documents, searchDocuments });
    const { getByTestId, getAllByTestId } = render(NotesPanel, { context });
    await fireEvent.input(getByTestId("notes-search"), { target: { value: "found" } });
    expect(searchDocuments).toHaveBeenCalledWith(
      "found",
      expect.objectContaining({ docTypes: ["note"] }),
      expect.any(Function),
    );
    const rows = getAllByTestId("note-row");
    expect(rows.map((r) => r.dataset.noteId)).toEqual(["n2"]);
  });
});

describe("NotesPanel create", () => {
  it("builds a private note owned by the caller and opens it, gated by canCreate", async () => {
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
    const { getByTestId } = render(NotesPanel, { context });
    await fireEvent.input(getByTestId("notes-name"), { target: { value: "Session 1" } });
    await fireEvent.click(getByTestId("notes-create"));
    expect(calls).toHaveLength(1);
    const created = (calls[0] as [{ op: string; doc: WireDocument }])[0];
    expect(created.op).toBe("create");
    expect(created.doc.name).toBe("Session 1");
    expect(created.doc.permissions.default).toBe("none");
    expect(created.doc.permissions.capabilities.by_role.owner).toEqual(
      expect.arrayContaining(["core:delete", "core:edit_permissions"]),
    );
    expect(opened).toEqual([{ docId: created.doc.id }]);
  });

  it("hides the create form when canCreate is false", () => {
    const context = setAppContextForTest({ documents: storeWith(), canCreate: () => false });
    const { queryByTestId } = render(NotesPanel, { context });
    expect(queryByTestId("notes-name")).toBeNull();
  });
});

describe("NotesPanel row gating", () => {
  it("shows Delete for a player whose fixture canDelete returns true, never keyed on raw owner", () => {
    const doc = buildNoteDoc("w1", "Shared", "", { id: "n1" }); // no owner stamped
    const context = setAppContextForTest({ documents: storeWith(doc), role: "player", canDelete: () => true });
    const { getByTestId } = render(NotesPanel, { context });
    expect(getByTestId("note-delete")).toBeTruthy();
  });

  it("hides Delete for a player whose fixture canDelete returns false", () => {
    const doc = buildNoteDoc("w1", "Shared", "", { id: "n1", owner: "someone-else" });
    const context = setAppContextForTest({ documents: storeWith(doc), role: "player", canDelete: () => false });
    const { queryByTestId } = render(NotesPanel, { context });
    expect(queryByTestId("note-delete")).toBeNull();
  });

  it("shows Move-to for a GM", () => {
    const doc = buildNoteDoc("w1", "A", "", { id: "n1" });
    const gmCtx = setAppContextForTest({ documents: storeWith(doc), role: "gm" });
    expect(render(NotesPanel, { context: gmCtx }).getByTestId("note-move")).toBeTruthy();
  });

  it("hides Move-to for a non-GM", () => {
    const doc = buildNoteDoc("w1", "A", "", { id: "n1" });
    const playerCtx = setAppContextForTest({ documents: storeWith(doc), role: "player", canDelete: () => true });
    expect(render(NotesPanel, { context: playerCtx }).queryByTestId("note-move")).toBeNull();
  });

  it("Move-to dispatches buildMoveOp's payload with the raw stored parent", async () => {
    const calls: unknown[] = [];
    const a = buildNoteDoc("w1", "A", "", { id: "n1" });
    const b = buildNoteDoc("w1", "B", "", { id: "n2" });
    const context = setAppContextForTest({
      documents: storeWith(a, b),
      role: "gm",
      dispatchIntent: (ops) => calls.push(ops),
    });
    const { getAllByTestId, getByTestId } = render(NotesPanel, { context });
    const moveButtons = getAllByTestId("note-move").filter((b) => b.dataset.noteId === "n1");
    await fireEvent.click(moveButtons[0]);
    await fireEvent.change(getByTestId("note-move-target"), { target: { value: "n2" } });
    expect(calls).toEqual([[{ op: "move", doc_id: "n1", parent_id: "n2", old_parent_id: null }]]);
  });
});
