import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { buildNoteDoc } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NoteTree from "./NoteTree.svelte";
import type { NoteTreeNode } from "./tree";

/** A rect for a 40px-tall row at `top`. */
function rowRect(top: number): DOMRect {
  return { top, height: 40, bottom: top + 40, left: 0, right: 100, width: 100, x: 0, y: top, toJSON: () => ({}) };
}

/** Three sibling notes sorted 1000/2000/3000 (or `sorts`), with their `.row` rects stubbed
 * 40px apart. */
function renderSiblings(sorts: number[], over: Record<string, unknown> = {}, reorderable = true) {
  const docs = sorts.map((sort, i) => buildNoteDoc("w1", "N" + i, "", { id: "n" + i, sort }));
  const dispatched: unknown[][] = [];
  const context = setAppContextForTest({ dispatchIntent: (ops: unknown[]) => dispatched.push(ops), ...over } as never);
  const result = render(NoteTree, {
    props: { nodes: docs.map((doc) => ({ doc, children: [] })), allNotes: docs, reorderable },
    context,
  });
  const rows = Array.from(result.container.querySelectorAll<HTMLElement>(".row"));
  rows.forEach((r, i) => (r.getBoundingClientRect = () => rowRect(i * 40)));
  return { ...result, dispatched, rows };
}

describe("NoteTree sibling reorder", () => {
  it("a pointer drag of the first note past the second's midpoint dispatches one Update on /engine/sort", async () => {
    const { getAllByTestId, dispatched } = renderSiblings([1000, 2000, 3000]);
    await fireEvent.pointerDown(getAllByTestId("note-drag-handle")[0], { clientY: 0 });
    await fireEvent(window, new PointerEvent("pointerup", { clientY: 70 })); // below mid(n1)=60
    expect(dispatched).toEqual([
      [{ op: "update", doc_id: "n0", changes: [{ path: "/engine/sort", old: 1000, new: 2500 }] }],
    ]);
  });

  it("Alt+ArrowDown dispatches byte-identical operations to the equivalent pointer drop", async () => {
    const drag = renderSiblings([1000, 2000, 3000]);
    await fireEvent.pointerDown(drag.getAllByTestId("note-drag-handle")[0], { clientY: 0 });
    await fireEvent(window, new PointerEvent("pointerup", { clientY: 70 }));
    drag.unmount();
    const key = renderSiblings([1000, 2000, 3000]);
    await fireEvent.keyDown(key.rows[0], { key: "ArrowDown", altKey: true });
    expect(key.dispatched).toEqual(drag.dispatched);
    expect(key.dispatched).toHaveLength(1);
  });

  it("an exhausted gap renumbers every sibling in one batched intent", async () => {
    const { rows, dispatched } = renderSiblings([1000, 1001, 1002]);
    await fireEvent.keyDown(rows[2], { key: "ArrowUp", altKey: true });
    expect(dispatched).toHaveLength(1);
    expect(dispatched[0]).toEqual([
      { op: "update", doc_id: "n2", changes: [{ path: "/engine/sort", old: 1002, new: 2000 }] },
      { op: "update", doc_id: "n1", changes: [{ path: "/engine/sort", old: 1001, new: 3000 }] },
    ]);
  });

  it("Alt+Arrow at either end of the siblings dispatches nothing", async () => {
    const { rows, dispatched } = renderSiblings([1000, 2000]);
    await fireEvent.keyDown(rows[0], { key: "ArrowUp", altKey: true });
    await fireEvent.keyDown(rows[1], { key: "ArrowDown", altKey: true });
    expect(dispatched).toEqual([]);
  });

  it("a plain arrow key without Alt never reorders", async () => {
    const { rows, dispatched } = renderSiblings([1000, 2000]);
    await fireEvent.keyDown(rows[0], { key: "ArrowDown" });
    expect(dispatched).toEqual([]);
  });

  it("a drag released on its own row dispatches nothing", async () => {
    const { getAllByTestId, dispatched } = renderSiblings([1000, 2000, 3000]);
    await fireEvent.pointerDown(getAllByTestId("note-drag-handle")[1], { clientY: 60 });
    await fireEvent(window, new PointerEvent("pointerup", { clientY: 62 }));
    expect(dispatched).toEqual([]);
  });

  it("without reorderable (a flat search-hit list) there is no handle and the keys do nothing", async () => {
    const { queryAllByTestId, rows, dispatched } = renderSiblings([1000, 2000], {}, false);
    expect(queryAllByTestId("note-drag-handle")).toHaveLength(0);
    await fireEvent.keyDown(rows[0], { key: "ArrowDown", altKey: true });
    expect(dispatched).toEqual([]);
  });

  it("a viewer who cannot edit the sort key gets neither the handle nor the keys", async () => {
    const { queryAllByTestId, rows, dispatched } = renderSiblings([1000, 2000], { canEdit: () => false });
    expect(queryAllByTestId("note-drag-handle")).toHaveLength(0);
    await fireEvent.keyDown(rows[0], { key: "ArrowDown", altKey: true });
    expect(dispatched).toEqual([]);
  });
});

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
