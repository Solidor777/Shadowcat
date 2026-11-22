<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, escapeCoordinator, ESCAPE_PRIORITY, trapFocus } from "@shadowcat/ui-kit";
  import {
    deleteAssetFolder,
    buildMoveOp,
    buildUpdate,
    buildReorderOps,
    createDragController,
    dropIndex,
    type DragTarget,
    type DropResult,
    type WireDocument,
  } from "@shadowcat/core";
  import {
    folderChildren,
    folderSiblings,
    buildFolderDoc,
    appendFolderSortKey,
    isDescendantOrSelf,
    ASSET_FOLDER_DOC_TYPE,
    ROOT_DROP_ID,
  } from "./folderOps";

  let {
    selectedFolder,
    onSelectFolder,
    onDropFiles,
    mutable = true,
    assetDropHover = null,
  }: {
    /** The folder filtering the grid, or `null` = all assets. */
    selectedFolder: string | null;
    /** Called when a node (or the all-assets row) is chosen. */
    onSelectFolder: (id: string | null) => void;
    /** OS files dropped onto a folder node (upload targets). */
    onDropFiles?: (files: File[], folderId: string) => void;
    /** Whether mutation affordances (create/move/delete) render; pick mode
     * passes `false`. Defaults to mutable. */
    mutable?: boolean;
    /** The drop resolution of an in-progress asset-tile drag (owned by the grid), used to
     * highlight the folder row or all-assets row it would land on. */
    assetDropHover?: DropResult | null;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  /** Each folder row's element by folder id: the drag controllers' geometry lookup. */
  const rowEls = new Map<string, HTMLElement>();
  /** The all-assets row's element, a drop target for asset tiles. */
  let allEl = $state<HTMLElement | null>(null);
  /** The drop resolution under the pointer during a folder drag, for the drop-indicator styling. */
  let folderHover = $state<DropResult | null>(null);

  /** `use:` action registering a folder row's element under its folder id.
   * @param node - The row's element.
   * @param id - The folder's document id.
   * @returns A teardown removing the registration.
   * @example
   * ```svelte
   * <div use:trackRow={"folder-id"}></div>
   * ```
   */
  function trackRow(node: HTMLElement, id: string): {
    /** Removes the registration on unmount. */
    destroy: () => void;
  } {
    rowEls.set(id, node);
    return { destroy: () => rowEls.delete(id) };
  }

  /** The drop targets an asset-tile drag lands on: every folder row plus the all-assets row.
   * The grid passes these to its drag controller (the tree and grid are siblings, so the tree
   * exposes them through `bind:this`).
   * @returns The targets, rects read fresh on each call.
   * @example
   * ```
   * // exported; invoked by AssetBrowser through bind:this
   * declare const tree: { dropTargets(): DragTarget[] };
   * tree.dropTargets();
   * ```
   */
  export function dropTargets(): DragTarget[] {
    const targets: DragTarget[] = folders.map((f) => ({
      id: f.id,
      rect: () => rowEls.get(f.id)?.getBoundingClientRect() ?? new DOMRect(),
    }));
    if (allEl) {
      const el = allEl;
      targets.push({ id: ROOT_DROP_ID, rect: () => el.getBoundingClientRect() });
    }
    return targets;
  }

  /** The ONE sibling-reorder dispatch both the pointer drop and Alt+Arrow keys call.
   * @param parentId - The shared parent (`null` = root).
   * @param from - The moved folder's index among its siblings.
   * @param to - Its destination index (post-removal convention).
   * @example
   * ```
   * // private function; invoked from the folder drag's onDrop and onRowKeydown
   * reorderFolder(null, 0, 1);
   * ```
   */
  function reorderFolder(parentId: string | null, from: number, to: number): void {
    const ops = buildReorderOps(folderSiblings(folders, parentId, ctx.compare), from, to);
    if (ops) ctx.dispatchIntent(ops);
  }

  const folderDrag = createDragController({
    axis: "vertical",
    getTargets: () => folders.map((f) => ({ id: f.id, rect: () => rowEls.get(f.id)?.getBoundingClientRect() ?? new DOMRect() })),
    overIsContainer: () => true,
    onHover: (result) => (folderHover = result),
    onDrop: (result) => {
      const dragged = folders.find((f) => f.id === result.dragged);
      const over = folders.find((f) => f.id === result.over);
      if (!dragged || !over) return;
      if (result.placement === "into") {
        if (!isDescendantOrSelf(folders, dragged.id, over.id)) moveFolder(dragged.id, over.id);
        return;
      }
      const parentId = dragged.parent_id ?? null;
      if ((over.parent_id ?? null) !== parentId) return;
      const ids = folderSiblings(folders, parentId, ctx.compare).map((s) => s.id);
      const to = dropIndex(ids, dragged.id, over.id, result.placement);
      if (to !== null) reorderFolder(parentId, ids.indexOf(dragged.id), to);
    },
  });

  /** Alt+ArrowUp/Down moves a folder one place among its siblings.
   * @param f - The folder whose row has focus.
   * @param ev - The keydown event.
   * @example
   * ```
   * // private function; wired to each folder row's onkeydown
   * declare const f: WireDocument, ev: KeyboardEvent;
   * onRowKeydown(f, ev);
   * ```
   */
  function onRowKeydown(f: WireDocument, ev: KeyboardEvent): void {
    if (!ev.altKey || !mutable || !ctx.canEdit(f, "/engine/sort")) return;
    const parentId = f.parent_id ?? null;
    const index = folderSiblings(folders, parentId, ctx.compare).findIndex((s) => s.id === f.id);
    const to = ev.key === "ArrowUp" ? index - 1 : ev.key === "ArrowDown" ? index + 1 : -1;
    if (to < 0 || to >= folderSiblings(folders, parentId, ctx.compare).length) return;
    ev.preventDefault();
    reorderFolder(parentId, index, to);
  }

  // Reactive bridge (mandatory): the tree re-renders on folder doc changes.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const folders = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query(ASSET_FOLDER_DOC_TYPE);
  });

  let createDraft = $state("");
  /** The folder being renamed inline, or null. */
  let renamingFor = $state<string | null>(null);
  let renameDraft = $state("");
  /** The folder whose Move-to picker is open, or null. */
  let movePickerFor = $state<string | null>(null);
  /** The folder whose delete dialog is open, or null. */
  let deleteDialogFor = $state<string | null>(null);
  /** The open inline rename input, bound while a rename is in progress. */
  let renameEl = $state<HTMLInputElement | null>(null);
  /** The open delete dialog element, bound while the dialog is shown. */
  let deleteDialogEl = $state<HTMLElement | null>(null);

  // Escape cancels an inline rename only when the keystroke comes from the rename input itself.
  $effect(() => {
    if (renamingFor === null) return;
    return escapeCoordinator.register(
      ESCAPE_PRIORITY.OVERLAY,
      () => {
        renamingFor = null;
      },
      (event) => renameEl !== null && event.target === renameEl,
    );
  });

  // The delete dialog closes on Escape, takes focus when it opens, and keeps Tab inside itself.
  $effect(() => {
    if (deleteDialogFor === null || deleteDialogEl === null) return;
    const release = trapFocus(deleteDialogEl);
    const unregister = escapeCoordinator.register(ESCAPE_PRIORITY.OVERLAY, () => {
      deleteDialogFor = null;
    });
    deleteDialogEl.focus();
    return () => {
      release();
      unregister();
    };
  });

  /** Commits the create input as a new root-level folder (or under the
   * current selection).
   * @example
   * ```
   * // private function; wired to the create input's Enter handler below
   * commitCreate();
   * ```
   */
  function commitCreate(): void {
    const name = createDraft.trim();
    createDraft = "";
    if (!name) return;
    const sort = appendFolderSortKey(folders, selectedFolder);
    ctx.dispatchIntent([
      { op: "create", doc: buildFolderDoc(ctx.world, name, selectedFolder, sort) },
    ]);
  }

  /** Commits the inline rename as a field-path Update on `/name`, carrying
   * the stored name as the OCC pre-image.
   * @param folder - The folder document being renamed.
   * @example
   * ```
   * // private function; wired to the rename input's Enter handler below
   * declare const folder: WireDocument;
   * commitRename(folder);
   * ```
   */
  function commitRename(folder: WireDocument): void {
    const next = renameDraft.trim();
    renamingFor = null;
    if (!next || next === folder.name) return;
    ctx.dispatchIntent([buildUpdate(folder.id, [{ path: "/name", old: folder.name, value: next }])]);
  }

  /** Dispatches the Move op for `id` toward `target`, with the stored parent
   * as the OCC pre-image.
   * @param id - The folder being moved.
   * @param target - The new parent (`null` = root).
   * @example
   * ```
   * // private function; wired to the Move-to picker below
   * moveFolder("some-folder", null);
   * ```
   */
  function moveFolder(id: string, target: string | null): void {
    movePickerFor = null;
    const current = folders.find((f) => f.id === id)?.parent_id ?? null;
    if (target === current) return;
    ctx.dispatchIntent([buildMoveOp(id, target, current)]);
  }

  /** Runs the folder delete with the dialog's chosen asset policy.
   * @param id - The folder to delete.
   * @param assets - `reparent` (default UX) or `delete` (purge).
   * @example
   * ```
   * // private function; wired to the delete dialog below
   * void confirmDelete("some-folder", "reparent");
   * ```
   */
  async function confirmDelete(id: string, assets: "reparent" | "delete"): Promise<void> {
    deleteDialogFor = null;
    try {
      await deleteAssetFolder(id, assets);
      if (selectedFolder === id) onSelectFolder(null);
    } catch (e) {
      ctx.notify(t("assetBrowser.error", { message: String(e) }));
    }
  }

  /** Handles OS files dropped on a folder node (upload targets). Dragged assets and folders
   * travel through the pointer drag controllers, not this native drop.
   * @param ev - The drop event.
   * @param folderId - The node dropped onto.
   * @example
   * ```
   * // private function; wired to each node's ondrop below
   * declare const ev: DragEvent;
   * handleDrop(ev, "some-folder");
   * ```
   */
  function handleDrop(ev: DragEvent, folderId: string): void {
    const files = Array.from(ev.dataTransfer?.files ?? []);
    if (files.length === 0) return;
    ev.preventDefault();
    onDropFiles?.(files, folderId);
  }
</script>

<div class="folder-tree">
  <button
    type="button"
    class="node all"
    class:selected={selectedFolder === null}
    class:drop-into={assetDropHover?.over === ROOT_DROP_ID}
    data-testid="folder-node-all"
    bind:this={allEl}
    onclick={() => onSelectFolder(null)}
  >{t("assetBrowser.allAssets")}</button>

  {#snippet nodes(parent: string | null, depth: number)}
    {#each folderChildren(folders, parent, ctx.compare) as f (f.id)}
      <div
        class="row"
        class:drop-before={folderHover?.over === f.id && folderHover.placement === "before"}
        class:drop-after={folderHover?.over === f.id && folderHover.placement === "after"}
        class:drop-into={(folderHover?.over === f.id && folderHover.placement === "into") || assetDropHover?.over === f.id}
        style:padding-left="{depth * 0.75}rem"
        use:trackRow={f.id}
        role="presentation"
        onkeydown={(e) => onRowKeydown(f, e)}
      >
        {#if mutable && ctx.canEdit(f, "/engine/sort")}
          <button
            type="button"
            class="mini drag-handle"
            data-testid={"folder-drag-" + f.id}
            aria-label={t("assetBrowser.dragFolder")}
            onpointerdown={(e) => folderDrag.beginDrag(f.id, e)}
          >⠿</button>
        {/if}
        <button
          type="button"
          class="node"
          class:selected={selectedFolder === f.id}
          data-testid={"folder-node-" + f.id}
          ondragover={(e) => {
            if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
          }}
          ondrop={(e) => handleDrop(e, f.id)}
          onclick={() => onSelectFolder(f.id)}
        >📁 {f.name}</button>
        {#if mutable}
          <button
            type="button"
            class="mini"
            data-testid={"folder-rename-" + f.id}
            title={t("assetBrowser.renameFolder")}
            onclick={() => {
              renamingFor = renamingFor === f.id ? null : f.id;
              renameDraft = f.name ?? "";
            }}
          >✎</button>
          <button
            type="button"
            class="mini"
            data-testid={"folder-move-" + f.id}
            title={t("assetBrowser.moveFolder")}
            onclick={() => (movePickerFor = movePickerFor === f.id ? null : f.id)}
          >⇄</button>
          <button
            type="button"
            class="mini"
            data-testid={"folder-delete-" + f.id}
            title={t("assetBrowser.deleteFolder")}
            onclick={() => (deleteDialogFor = f.id)}
          >🗑</button>
        {/if}
      </div>
      {#if renamingFor === f.id}
        <input
          data-testid={"folder-rename-input-" + f.id}
          type="text"
          bind:value={renameDraft}
          bind:this={renameEl}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitRename(f);
            }
          }}
        />
      {/if}
      {#if movePickerFor === f.id}
        <div class="move-picker" role="menu">
          <button type="button" data-testid="folder-move-target-root" onclick={() => moveFolder(f.id, null)}>
            {t("assetBrowser.moveToRoot")}
          </button>
          {#each folders.filter((x) => !isDescendantOrSelf(folders, f.id, x.id)) as target (target.id)}
            <button
              type="button"
              data-testid={"folder-move-target-" + target.id}
              onclick={() => moveFolder(f.id, target.id)}
            >{target.name}</button>
          {/each}
        </div>
      {/if}
      {#if deleteDialogFor === f.id}
        <div
          class="delete-dialog"
          role="alertdialog"
          tabindex="-1"
          aria-label={t("assetBrowser.deleteFolder")}
          bind:this={deleteDialogEl}
        >
          <p>{t("assetBrowser.deleteFolderPrompt", { name: f.name ?? "" })}</p>
          <button type="button" data-testid="folder-delete-reparent" onclick={() => void confirmDelete(f.id, "reparent")}>
            {t("assetBrowser.deleteReparent")}
          </button>
          <button type="button" class="danger" data-testid="folder-delete-purge" onclick={() => void confirmDelete(f.id, "delete")}>
            {t("assetBrowser.deletePurge")}
          </button>
          <button type="button" onclick={() => (deleteDialogFor = null)}>
            {t("assetBrowser.pickCancel")}
          </button>
        </div>
      {/if}
      {@render nodes(f.id, depth + 1)}
    {/each}
  {/snippet}
  {@render nodes(null, 0)}

  {#if mutable}
    <input
      data-testid="folder-create-name"
      type="text"
      placeholder={t("assetBrowser.newFolder")}
      bind:value={createDraft}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commitCreate();
        }
      }}
    />
  {/if}
</div>

<style lang="scss">
  .folder-tree {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    padding: 0.375rem;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 0.125rem;
  }
  .node {
    flex: 1;
    text-align: left;
    min-height: 2rem;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--text-primary);
    &.selected {
      background: var(--surface-raised);
      outline: 1px solid var(--accent);
    }
  }
  .row.drop-before {
    box-shadow: 0 -2px 0 var(--accent);
  }
  .row.drop-after {
    box-shadow: 0 2px 0 var(--accent);
  }
  .row.drop-into,
  .node.all.drop-into {
    outline: 2px solid var(--accent);
  }
  .mini.drag-handle {
    min-width: 2.75rem;
    user-select: none;
  }
  .mini {
    min-width: 1.75rem;
    min-height: 1.75rem;
    border: none;
    background: none;
    cursor: pointer;
  }
  .move-picker,
  .delete-dialog {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    margin: 0.25rem 0 0.25rem 1rem;
    padding: 0.375rem;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    button {
      min-height: 2rem;
      text-align: left;
    }
  }
  .delete-dialog .danger {
    color: var(--danger);
  }
  input {
    min-height: 2rem;
    margin-top: 0.375rem;
  }
</style>
