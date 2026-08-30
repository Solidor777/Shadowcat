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

const PROPS = { selectedFolder: null as string | null, onSelectFolder: () => {}, onDropAssets: () => {} };

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

test("dropping dragged assets on a node reports them with the folder id", async () => {
  const onDropAssets = vi.fn();
  render(FolderTree, { props: { ...PROPS, onDropAssets }, context: treeCtx() });
  const node = await screen.findByTestId("folder-node-fa");
  const dataTransfer = {
    getData: (type: string) =>
      type === "application/x-shadowcat-assets" ? JSON.stringify(["a1", "a2"]) : "",
    types: ["application/x-shadowcat-assets"],
  };
  await fireEvent.drop(node, { dataTransfer });
  expect(onDropAssets).toHaveBeenCalledWith(["a1", "a2"], "fa");
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
