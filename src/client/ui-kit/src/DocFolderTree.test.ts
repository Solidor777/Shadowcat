import { describe, it, expect, vi, afterEach } from "vitest";
import { render, fireEvent, screen, cleanup, waitFor } from "@testing-library/svelte";
import { DocumentStore } from "@shadowcat/core";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import DocFolderTree from "./DocFolderTree.svelte";
import { buildTypedFolderDoc, type FolderTreeLabels, type FolderDeleteChoice } from "./folderTree";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** A folder doc type distinct from any shipped one, so the tests pin the parametrization. */
const KIND = "probe_folder";
const LABELS: FolderTreeLabels = {
  all: "probe.all",
  newFolder: "probe.new",
  dragFolder: "probe.drag",
  rename: "probe.rename",
  move: "probe.move",
  delete: "probe.delete",
  moveToRoot: "probe.root",
  deletePrompt: "probe.prompt",
  cancel: "probe.cancel",
  error: "probe.error",
};
const CHOICES: FolderDeleteChoice[] = [{ id: "go", labelKey: "probe.go", danger: true }];

/** A store holding probe folders `pa` (root) and `pb` (under `pa`), plus an `asset_folder` that
 * the probe tree must ignore. */
function seeded(): DocumentStore {
  const store = new DocumentStore();
  const mk = (id: string, type: string, name: string, parent: string | null) => ({
    ...buildTypedFolderDoc("w1", type, name, parent),
    id,
  });
  store.seedDocuments([mk("pa", KIND, "alpha", null), mk("pb", KIND, "beta", "pa"), mk("af", "asset_folder", "other", null)]);
  return store;
}

/** Renders the tree for the probe folder type. */
function mount(over: Record<string, unknown> = {}, props: Record<string, unknown> = {}) {
  const store = seeded();
  const context = setAppContextForTest({ documents: store, world: "w1", ...over } as never);
  return render(DocFolderTree, {
    props: {
      folderDocType: KIND,
      labels: LABELS,
      rootDropId: "probe-all",
      deleteChoices: CHOICES,
      onDelete: () => {},
      selectedFolder: null as string | null,
      onSelectFolder: () => {},
      ...props,
    },
    context,
  });
}

describe("DocFolderTree parametrization", () => {
  it("renders only folders of its own doc type, using its own label keys", async () => {
    mount();
    expect(await screen.findByTestId("folder-node-pa")).toBeTruthy();
    expect(screen.getByTestId("folder-node-pb")).toBeTruthy();
    expect(screen.queryByTestId("folder-node-af")).toBeNull();
    expect(screen.getByTestId("folder-node-all").textContent).toBe("probe.all");
    expect((screen.getByTestId("folder-create-name") as HTMLInputElement).placeholder).toBe("probe.new");
  });

  it("dropTargets lists every folder row plus the supplied root id", () => {
    const { component } = mount();
    const ids = (component as unknown as { dropTargets(): { id: string }[] }).dropTargets().map((t) => t.id);
    expect(ids.sort()).toEqual(["pa", "pb", "probe-all"]);
  });

  it("creating a folder dispatches a Create of the tree's doc type under the selection", async () => {
    const dispatchIntent = vi.fn();
    mount({ dispatchIntent }, { selectedFolder: "pa" });
    const input = screen.getByTestId("folder-create-name");
    await fireEvent.input(input, { target: { value: "gamma" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    const op = dispatchIntent.mock.calls[0][0][0];
    expect(op.op).toBe("create");
    expect(op.doc.doc_type).toBe(KIND);
    expect(op.doc.parent_id).toBe("pa");
    expect(op.doc.name).toBe("gamma");
  });

  it("a supplied buildFolder makes the create document", async () => {
    const dispatchIntent = vi.fn();
    const buildFolder = vi.fn((world: string, name: string, parentId: string | null, sort: number) => ({
      ...buildTypedFolderDoc(world, KIND, name, parentId, sort),
      id: "custom",
    }));
    mount({ dispatchIntent }, { buildFolder });
    const input = screen.getByTestId("folder-create-name");
    await fireEvent.input(input, { target: { value: "gamma" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(buildFolder).toHaveBeenCalledWith("w1", "gamma", null, expect.any(Number));
    expect(dispatchIntent.mock.calls[0][0][0].doc.id).toBe("custom");
  });

  it("the delete dialog renders the supplied choices and reports the chosen one with the folder", async () => {
    const onDelete = vi.fn();
    mount({}, { onDelete });
    await fireEvent.click(await screen.findByTestId("folder-delete-pb"));
    await fireEvent.click(screen.getByTestId("folder-delete-go"));
    await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(1));
    expect(onDelete.mock.calls[0][0].id).toBe("pb");
    expect(onDelete.mock.calls[0][1]).toBe("go");
  });

  it("a rejected delete notifies with the supplied error key", async () => {
    const notify = vi.fn();
    mount({ notify }, { onDelete: () => Promise.reject(new Error("boom")) });
    await fireEvent.click(await screen.findByTestId("folder-delete-pb"));
    await fireEvent.click(screen.getByTestId("folder-delete-go"));
    await waitFor(() => expect(notify).toHaveBeenCalledTimes(1));
    expect(notify.mock.calls[0][0]).toContain("probe.error");
  });

  it("deleting the selected folder resets the selection to all", async () => {
    const onSelectFolder = vi.fn();
    mount({}, { selectedFolder: "pb", onSelectFolder });
    await fireEvent.click(await screen.findByTestId("folder-delete-pb"));
    await fireEvent.click(screen.getByTestId("folder-delete-go"));
    await waitFor(() => expect(onSelectFolder).toHaveBeenCalledWith(null));
  });

  it("the Move-to picker dispatches a Move with the stored parent as the pre-image", async () => {
    const dispatchIntent = vi.fn();
    mount({ dispatchIntent });
    await fireEvent.click(await screen.findByTestId("folder-move-pb"));
    await fireEvent.click(screen.getByTestId("folder-move-target-root"));
    expect(dispatchIntent).toHaveBeenCalledWith([{ op: "move", doc_id: "pb", parent_id: null, old_parent_id: "pa" }]);
  });

  it("a drop-hover on a folder row highlights it", async () => {
    mount({}, { dropHover: { dragged: "x", over: "pa", placement: "into" } });
    const row = (await screen.findByTestId("folder-node-pa")).closest(".row") as HTMLElement;
    expect(row.classList.contains("drop-into")).toBe(true);
  });

  it("right-click on a row opens the menu with the supplied labels", async () => {
    mount();
    await fireEvent.contextMenu(await screen.findByTestId("folder-node-pb"));
    expect(screen.getByTestId("context-menu-item-rename")).toBeTruthy();
    expect(screen.getByTestId("context-menu-item-move")).toBeTruthy();
    expect(screen.getByTestId("context-menu-item-delete")).toBeTruthy();
  });
});
