import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { buildNoteDoc } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NoteTree from "./NoteTree.svelte";
import type { NoteTreeNode } from "./tree";

describe("NoteTree expand/collapse", () => {
  it("hides a node's children until its toggle is clicked", async () => {
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1" });
    const child = buildNoteDoc("w1", "Child", "", { id: "c1", parentId: "p1" });
    const nodes: NoteTreeNode[] = [{ doc: parent, children: [{ doc: child, children: [] }] }];
    const context = setAppContextForTest({});
    const { queryByTestId, getByTestId, getAllByTestId } = render(NoteTree, {
      props: { nodes, allNotes: [parent, child] },
      context,
    });
    expect(queryByTestId("note-row")).not.toBeNull();
    expect(getAllByTestId("note-row")).toHaveLength(1);
    await fireEvent.click(getByTestId("note-toggle"));
    expect(getAllByTestId("note-row")).toHaveLength(2);
  });

  it("opens a note's sheet on click", async () => {
    const opened: unknown[] = [];
    const doc = buildNoteDoc("w1", "A", "", { id: "n1" });
    const context = setAppContextForTest({ openDocument: (ref) => opened.push(ref) });
    const { getByTestId } = render(NoteTree, {
      props: { nodes: [{ doc, children: [] }], allNotes: [doc] },
      context,
    });
    await fireEvent.click(getByTestId("note-open"));
    expect(opened).toEqual([{ docId: "n1" }]);
  });
});
