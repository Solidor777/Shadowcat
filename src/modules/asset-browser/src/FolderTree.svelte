<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { deleteAssetFolder, type WireDocument } from "@shadowcat/core";
  import {
    folderChildren,
    buildMoveOp,
    buildFolderDoc,
    isDescendantOrSelf,
    ASSET_FOLDER_DOC_TYPE,
  } from "./folderOps";

  let {
    selectedFolder,
    onSelectFolder,
    onDropAssets,
    mutable = true,
  }: {
    /** The folder filtering the grid, or `null` = all assets. */
    selectedFolder: string | null;
    /** Called when a node (or the all-assets row) is chosen. */
    onSelectFolder: (id: string | null) => void;
    /** Dragged asset ids dropped onto a folder node. */
    onDropAssets: (ids: string[], folderId: string) => void;
    /** Whether mutation affordances (create/move/delete) render; pick mode
     * passes `false`. Defaults to mutable. */
    mutable?: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

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
    ctx.dispatchIntent([
      { op: "create", doc: buildFolderDoc(ctx.world, name, selectedFolder) },
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
    ctx.dispatchIntent([
      {
        op: "update",
        doc_id: folder.id,
        changes: [{ path: "/name", old: folder.name, new: next }],
      },
    ]);
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

  /** Handles a drop on a folder node: dragged assets file into it; a dragged
   * folder re-parents under it (subtree drops are refused client-side; the
   * server's cycle walk stays authoritative).
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
    ev.preventDefault();
    const assetPayload = ev.dataTransfer?.getData("application/x-shadowcat-assets");
    if (assetPayload) {
      try {
        const ids = JSON.parse(assetPayload) as string[];
        if (Array.isArray(ids) && ids.length > 0) onDropAssets(ids, folderId);
      } catch {
        // Malformed drag payload: ignore.
      }
      return;
    }
    const draggedFolder = ev.dataTransfer?.getData("application/x-shadowcat-folder");
    if (draggedFolder && !isDescendantOrSelf(folders, draggedFolder, folderId)) {
      moveFolder(draggedFolder, folderId);
    }
  }
</script>

<div class="folder-tree">
  <button
    type="button"
    class="node all"
    class:selected={selectedFolder === null}
    data-testid="folder-node-all"
    onclick={() => onSelectFolder(null)}
  >{t("assetBrowser.allAssets")}</button>

  {#snippet nodes(parent: string | null, depth: number)}
    {#each folderChildren(folders, parent) as f (f.id)}
      <div class="row" style:padding-left="{depth * 0.75}rem">
        <button
          type="button"
          class="node"
          class:selected={selectedFolder === f.id}
          data-testid={"folder-node-" + f.id}
          draggable="true"
          ondragstart={(e) => e.dataTransfer?.setData("application/x-shadowcat-folder", f.id)}
          ondragover={(e) => e.preventDefault()}
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
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitRename(f);
            } else if (e.key === "Escape") {
              renamingFor = null;
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
        <div class="delete-dialog" role="alertdialog" aria-label={t("assetBrowser.deleteFolder")}>
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
      outline: 1px solid var(--accent, #46f);
    }
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
    color: var(--danger, #c33);
  }
  input {
    min-height: 2rem;
    margin-top: 0.375rem;
  }
</style>
