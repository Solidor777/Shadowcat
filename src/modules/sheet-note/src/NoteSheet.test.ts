import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import {
  DocumentStore,
  buildNoteDoc,
  buildChannelRegistryDoc,
  resolveCaps,
  canWritePath,
  type WireDocument,
} from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NoteSheet from "./NoteSheet.svelte";

const SELF = "u-self";

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

describe("NoteSheet title + body", () => {
  it("edits the title with the real pre-image", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("note-title"), { target: { value: "Session 1 recap" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "n1", changes: [{ path: "/name", old: "Session 1", new: "Session 1 recap" }] }]]);
  });

  it("renders the server-derived body via SegmentList when it parses", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    doc.engine = { ...(doc.engine as object), body: [{ kind: "text", text: "Hello world" }] };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("Hello world")).toBeTruthy();
  });

  it("shows the unrenderable message when engine.body fails to parse", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    // The segment list schema tolerates an unrecognized `kind` (forward-compat); a genuine
    // parse failure needs the wrong TOP-LEVEL shape (not an array at all).
    doc.engine = { ...(doc.engine as object), body: "not-an-array" };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("sheetNote.unrenderable")).toBeTruthy();
  });
});

describe("NoteSheet draft-base edit flow", () => {
  it("Save dispatches ONE setField carrying the ORIGINAL source as old, even after a remote mutation", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.click(getByTestId("note-edit"));

    // Simulate a remote authoritative confirm changing the stored source WHILE the editor is open.
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });

    await fireEvent.input(getByTestId("note-source"), { target: { value: "# My edit" } });
    await fireEvent.click(getByTestId("note-save"));

    expect(calls).toEqual([
      [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# My edit" }] }],
    ]);
  });

  it("shows the changedRemotely banner when the stored source diverges from the draft base", async () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId, getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });
    await tick();
    expect(getByText("sheetNote.changedRemotely")).toBeTruthy();
  });

  it("Cancel discards the draft without dispatching", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId, queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    await fireEvent.input(getByTestId("note-source"), { target: { value: "# Discarded" } });
    await fireEvent.click(getByTestId("note-cancel"));
    expect(calls).toEqual([]);
    expect(queryByTestId("note-source")).toBeNull();
  });

  it("Reload re-seeds the draft from the current stored value", async () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId, getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });
    await fireEvent.click(getByText("sheetNote.reload"));
    expect((getByTestId("note-source") as HTMLTextAreaElement).value).toBe("# Remotely changed");
  });
});

describe("NoteSheet visibility", () => {
  it("shows the visibility select for the author (grantAuthor's core:edit_permissions), gated by the REAL resolveCaps + canWritePath", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      dispatchIntent: (ops) => calls.push(ops),
      role: "player",
      selfId: SELF,
      canEdit: (d, path) => canWritePath(path, resolveCaps(d.permissions, SELF, "player", { by_role: {}, by_user: {} }), false, []),
    });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("note-visibility"), { target: { value: "observer" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "n1", changes: [{ path: "/permissions/default", old: "none", new: "observer" }] }],
    ]);
  });

  it("hides the visibility select for a non-author observer, using the same real gate", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    doc.permissions.default = "observer";
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      role: "player",
      selfId: "someone-else",
      canEdit: (d, path) => canWritePath(path, resolveCaps(d.permissions, "someone-else", "player", { by_role: {}, by_user: {} }), false, []),
    });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-visibility")).toBeNull();
  });
});

describe("NoteSheet tree navigation", () => {
  it("lists children sorted by (engine.sort, created_at) and opens one on click", async () => {
    const opened: unknown[] = [];
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const childB = buildNoteDoc("w1", "B", "", { id: "cb", parentId: "p1", sort: 1 });
    const childA = buildNoteDoc("w1", "A", "", { id: "ca", parentId: "p1", sort: 0 });
    const documents = storeWith(parent, childB, childA, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true, openDocument: (ref) => opened.push(ref) });
    const { getAllByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    const rows = getAllByTestId("note-child");
    expect(rows.map((r) => r.textContent)).toEqual(["A", "B"]);
    await fireEvent.click(rows[0]);
    expect(opened).toEqual([{ docId: "ca" }]);
  });

  it("New child note creates a child and opens it, gated by canCreate", async () => {
    const calls: unknown[] = [];
    const opened: unknown[] = [];
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const documents = storeWith(parent, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      canEdit: () => true,
      canCreate: () => true,
      selfId: SELF,
      world: "w1",
      dispatchIntent: (ops) => calls.push(ops),
      openDocument: (ref) => opened.push(ref),
    });
    const { getByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-new-child"));
    expect(calls).toHaveLength(1);
    const created = (calls[0] as [{ op: string; doc: WireDocument }])[0];
    expect(created.op).toBe("create");
    expect(created.doc.parent_id).toBe("p1");
    expect(opened).toEqual([{ docId: created.doc.id }]);
  });

  it("hides New child note when canCreate is false", () => {
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const documents = storeWith(parent, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true, canCreate: () => false });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-new-child")).toBeNull();
  });
});
