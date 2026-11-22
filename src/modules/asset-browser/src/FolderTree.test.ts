import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore } from "@shadowcat/core";
import * as api from "@shadowcat/core";
import FolderTree from "./FolderTree.svelte";
import { buildFolderDoc } from "./folderOps";

beforeEach(() => vi.restoreAllMocks());

function seededStore() {
  const store = new DocumentStore();
  const a = { ...buildFolderDoc("w1", "alpha", null), id: "fa" };
  const b = { ...buildFolderDoc("w1", "beta", "fa"), id: "fb" };
  store.seedDocuments([a, b]);
  return store;
}

function treeCtx(over: Record<string, unknown> = {}) {
  const store = seededStore();
  return setAppContextForTest({ store, documents: store, ...over } as never);
}

const PROPS = { selectedFolder: null as string | null, onSelectFolder: () => {} };

test("renders nested folders from the store", async () => {
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  const parent = await screen.findByTestId("folder-node-fa");
  const child = await screen.findByTestId("folder-node-fb");
  expect(parent.textContent).toContain("alpha");
  expect(child.textContent).toContain("beta");
});

test("the accessible move control dispatches a Move op with the true pre-image", async () => {
  const dispatchIntent = vi.fn();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx({ dispatchIntent }) });
  // Move "beta" (child of fa) to the root via its Move-to picker.
  await fireEvent.click(await screen.findByTestId("folder-move-fb"));
  await fireEvent.click(await screen.findByTestId("folder-move-target-root"));
  expect(dispatchIntent).toHaveBeenCalledWith([
    { op: "move", doc_id: "fb", parent_id: null, old_parent_id: "fa" },
  ]);
});

test("the move picker excludes the folder's own subtree", async () => {
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  await fireEvent.click(await screen.findByTestId("folder-move-fa"));
  // Targets: root and OTHER folders only — never itself or its descendant.
  expect(screen.queryByTestId("folder-move-target-fa")).toBeNull();
  expect(screen.queryByTestId("folder-move-target-fb")).toBeNull();
  expect(screen.getByTestId("folder-move-target-root")).toBeTruthy();
});

test("the delete dialog maps its choice onto the assets policy param", async () => {
  const del = vi.spyOn(api, "deleteAssetFolder").mockResolvedValue();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  await fireEvent.click(await screen.findByTestId("folder-delete-fb"));
  await fireEvent.click(screen.getByTestId("folder-delete-purge"));
  await waitFor(() => expect(del).toHaveBeenCalledWith("fb", "delete"));

  await fireEvent.click(await screen.findByTestId("folder-delete-fa"));
  await fireEvent.click(screen.getByTestId("folder-delete-reparent"));
  await waitFor(() => expect(del).toHaveBeenCalledWith("fa", "reparent"));
});

test("Escape closes the delete dialog without deleting", async () => {
  const del = vi.spyOn(api, "deleteAssetFolder").mockResolvedValue();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  await fireEvent.click(await screen.findByTestId("folder-delete-fb"));
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  await fireEvent.keyDown(screen.getByRole("alertdialog"), { key: "Escape" });
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(del).not.toHaveBeenCalled();
});

test("the delete dialog takes focus and Tab from its last button wraps to its first", async () => {
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  await fireEvent.click(await screen.findByTestId("folder-delete-fb"));
  const dialog = screen.getByRole("alertdialog");
  await waitFor(() => expect(document.activeElement).toBe(dialog));
  const buttons = dialog.querySelectorAll("button");
  const last = buttons[buttons.length - 1] as HTMLElement;
  last.focus();
  await fireEvent.keyDown(last, { key: "Tab" });
  expect(document.activeElement).toBe(buttons[0]);
});

test("Escape in the rename input cancels the rename without dispatching", async () => {
  const dispatchIntent = vi.fn();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx({ dispatchIntent }) });
  await fireEvent.click(await screen.findByTestId("folder-rename-fb"));
  const input = screen.getByTestId("folder-rename-input-fb");
  await fireEvent.keyDown(input, { key: "Escape" });
  expect(screen.queryByTestId("folder-rename-input-fb")).toBeNull();
  expect(dispatchIntent).not.toHaveBeenCalled();
});

test("Escape outside the rename input leaves the rename open and unconsumed", async () => {
  render(FolderTree, { props: { ...PROPS }, context: treeCtx() });
  await fireEvent.click(await screen.findByTestId("folder-rename-fb"));
  expect(await fireEvent.keyDown(document.body, { key: "Escape" })).toBe(true);
  expect(screen.getByTestId("folder-rename-input-fb")).toBeTruthy();
});

test("OS files dropped on a node report with the folder id", async () => {
  const onDropFiles = vi.fn();
  render(FolderTree, { props: { ...PROPS, onDropFiles }, context: treeCtx() });
  const node = await screen.findByTestId("folder-node-fa");
  const file = new File(["x"], "a.png", { type: "image/png" });
  await fireEvent.drop(node, { dataTransfer: { files: [file], types: ["Files"] } });
  expect(onDropFiles).toHaveBeenCalledWith([file], "fa");
});

/** A 40px-tall rect at `top`. */
function rowRect(top: number): DOMRect {
  return { top, height: 40, bottom: top + 40, left: 0, right: 100, width: 100, x: 0, y: top, toJSON: () => ({}) };
}

/** Three root folders r0/r1/r2 (sorts 1000/2000/3000) with one child c0 under r0. Rows render
 * depth-first: r0, c0, r1, r2, stubbed 40px apart from the top. */
function renderSiblings(over: Record<string, unknown> = {}, props: Record<string, unknown> = {}) {
  const store = new DocumentStore();
  const mk = (id: string, parent: string | null, sort: number) => ({ ...buildFolderDoc("w1", id, parent, sort), id });
  store.seedDocuments([mk("r0", null, 1000), mk("c0", "r0", 1000), mk("r1", null, 2000), mk("r2", null, 3000)]);
  const dispatched: unknown[][] = [];
  const context = setAppContextForTest({ store, documents: store, dispatchIntent: (ops: unknown[]) => dispatched.push(ops), ...over } as never);
  const result = render(FolderTree, { props: { ...PROPS, ...props }, context });
  const tops: Record<string, number> = { r0: 0, c0: 40, r1: 80, r2: 120 };
  const rowOf = (id: string) => result.getByTestId("folder-node-" + id).closest(".row") as HTMLElement;
  for (const id of Object.keys(tops)) rowOf(id).getBoundingClientRect = () => rowRect(tops[id]);
  return { ...result, dispatched, rowOf };
}

/** Drags folder `id`'s handle and releases at `y`. */
async function dragFolder(id: string, y: number): Promise<void> {
  await fireEvent.pointerDown(screen.getByTestId("folder-drag-" + id), { clientY: 0 });
  await fireEvent(window, new PointerEvent("pointerup", { clientX: 10, clientY: y }));
}

test("dropping a folder on the middle of another folder moves it into that folder with the stored pre-image", async () => {
  const { dispatched } = renderSiblings();
  await dragFolder("r0", 100); // middle of r1 (80..120)
  expect(dispatched).toEqual([[{ op: "move", doc_id: "r0", parent_id: "r1", old_parent_id: null }]]);
});

test("dropping a folder into its own descendant is refused", async () => {
  const { dispatched } = renderSiblings();
  await dragFolder("r0", 60); // middle of c0, r0's child
  expect(dispatched).toEqual([]);
});

test("dropping a folder on a sibling's edge reorders by one Update on /engine/sort", async () => {
  const { dispatched } = renderSiblings();
  await dragFolder("r0", 155); // lower quarter of r2: after r2
  expect(dispatched).toEqual([
    [{ op: "update", doc_id: "r0", changes: [{ path: "/engine/sort", old: 1000, new: 4000 }] }],
  ]);
});

test("dropping on the edge of a folder that is not a sibling does nothing", async () => {
  const { dispatched } = renderSiblings();
  await dragFolder("r1", 42); // upper quarter of c0, a child of r0
  expect(dispatched).toEqual([]);
});

test("Alt+ArrowDown dispatches byte-identical operations to the equivalent pointer drop", async () => {
  const drag = renderSiblings();
  await dragFolder("r0", 115); // lower quarter of r1: after r1
  drag.unmount();
  const key = renderSiblings();
  await fireEvent.keyDown(key.rowOf("r0"), { key: "ArrowDown", altKey: true });
  expect(key.dispatched).toEqual(drag.dispatched);
  expect(key.dispatched).toHaveLength(1);
});

test("Alt+Arrow past either end of the siblings dispatches nothing", async () => {
  const { dispatched, rowOf } = renderSiblings();
  await fireEvent.keyDown(rowOf("r0"), { key: "ArrowUp", altKey: true });
  await fireEvent.keyDown(rowOf("r2"), { key: "ArrowDown", altKey: true });
  expect(dispatched).toEqual([]);
});

test("pick mode (not mutable) renders no drag handle and ignores the reorder keys", async () => {
  const { queryByTestId, dispatched, rowOf } = renderSiblings({}, { mutable: false });
  expect(queryByTestId("folder-drag-r0")).toBeNull();
  await fireEvent.keyDown(rowOf("r0"), { key: "ArrowDown", altKey: true });
  expect(dispatched).toEqual([]);
});

test("a viewer who cannot edit a folder's sort key gets no drag handle for it", async () => {
  const { queryByTestId } = renderSiblings({ canEdit: () => false });
  expect(queryByTestId("folder-drag-r0")).toBeNull();
});

test("dropTargets lists every folder row and the all-assets row", async () => {
  const { component } = renderSiblings();
  const ids = (component as unknown as { dropTargets(): { id: string }[] }).dropTargets().map((t) => t.id);
  expect(ids.sort()).toEqual(["all-assets", "c0", "r0", "r1", "r2"]);
});

test("creating a folder dispatches a Create with the asset_folder envelope", async () => {
  const dispatchIntent = vi.fn();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx({ dispatchIntent }) });
  const input = screen.getByTestId("folder-create-name");
  await fireEvent.input(input, { target: { value: "maps" } });
  await fireEvent.keyDown(input, { key: "Enter" });
  expect(dispatchIntent).toHaveBeenCalledTimes(1);
  const ops = dispatchIntent.mock.calls[0][0];
  expect(ops[0].op).toBe("create");
  expect(ops[0].doc.doc_type).toBe("asset_folder");
  expect(ops[0].doc.name).toBe("maps");
});

test("inline rename dispatches an Update on /name with the stored pre-image", async () => {
  const dispatchIntent = vi.fn();
  render(FolderTree, { props: { ...PROPS }, context: treeCtx({ dispatchIntent }) });
  await fireEvent.click(await screen.findByTestId("folder-rename-fb"));
  const input = screen.getByTestId("folder-rename-input-fb");
  await fireEvent.input(input, { target: { value: "bravo" } });
  await fireEvent.keyDown(input, { key: "Enter" });
  expect(dispatchIntent).toHaveBeenCalledWith([
    {
      op: "update",
      doc_id: "fb",
      changes: [{ path: "/name", old: "beta", new: "bravo" }],
    },
  ]);
});
