<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import {
    buildMoveOp,
    buildUpdate,
    buildReorderOps,
    createDragController,
    dropIndex,
    type DragTarget,
    type DropResult,
    type WireDocument,
  } from "@shadowcat/core";
  import { getAppContext } from "./appContext";
  import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";
  import { trapFocus } from "./focusTrap";
  import ContextMenu from "./ContextMenu.svelte";
  import { createContextMenuTrigger } from "./contextMenu";
  import type { ContextMenuItem, ContextMenuRequest, ContextMenuTrigger } from "./contextMenu";
  import {
    typedFolderChildren,
    typedFolderSiblings,
    appendTypedFolderSortKey,
    buildTypedFolderDoc,
    isDescendantOrSelf,
    type FolderTreeLabels,
    type FolderDeleteChoice,
  } from "./folderTree";

  let {
    folderDocType,
    labels,
    rootDropId,
    deleteChoices,
    onDelete,
    buildFolder = (world, name, parentId, sort) => buildTypedFolderDoc(world, folderDocType, name, parentId, sort),
    selectedFolder,
    onSelectFolder,
    onDropFiles,
    mutable = true,
    dropHover = null,
  }: {
    /** The folder `doc_type` this tree browses and creates. */
    folderDocType: string;
    /** The i18n keys the tree renders. */
    labels: FolderTreeLabels;
    /** The drop-target id of the all-items row: an item dropped on it leaves its folder. Never
     * collides with a folder id, which is a UUID. */
    rootDropId: string;
    /** The delete dialog's buttons, in order. */
    deleteChoices: FolderDeleteChoice[];
    /** Runs the owner's delete for the chosen button; a rejection is reported through
     * `labels.error`. */
    onDelete: (folder: WireDocument, choiceId: string) => Promise<void> | void;
    /** Builds the document a create-folder commit dispatches; defaults to a plain envelope of
     * `folderDocType`. A folder type whose creator needs author grants supplies its own. */
    buildFolder?: (world: string, name: string, parentId: string | null, sort: number) => WireDocument;
    /** The folder filtering the item list, or `null` = all items. */
    selectedFolder: string | null;
    /** Called when a node (or the all-items row) is chosen. */
    onSelectFolder: (id: string | null) => void;
    /** OS files dropped onto a folder node (upload targets). */
    onDropFiles?: (files: File[], folderId: string) => void;
    /** Whether mutation affordances (create/move/delete) render; pick mode passes `false`. */
    mutable?: boolean;
    /** The drop resolution of an in-progress item drag (owned by the item list), used to
     * highlight the folder row or all-items row it would land on. */
    dropHover?: DropResult | null;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  /** Each folder row's element by folder id: the drag controllers' geometry lookup. */
  const rowEls = new Map<string, HTMLElement>();
  /** The all-items row's element, a drop target for items. */
  let allEl = $state<HTMLElement | null>(null);
  /** The drop resolution under the pointer during a folder drag, for the drop-indicator styling. */
  let folderHover = $state<DropResult | null>(null);

  /** `use:` action registering a folder row's element under its folder id.
   * @param node - The row's element.
   * @param id - The folder's document id.
   * @returns A teardown removing the registration and the row's context-menu trigger.
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
    return {
      destroy: () => {
        rowEls.delete(id);
        rowTriggers.delete(id);
      },
    };
  }

  /** The drop targets an item drag lands on: every folder row plus the all-items row. The item
   * list passes these to its drag controller (the tree and list are siblings, so the tree
   * exposes them through `bind:this`).
   * @returns The targets, rects read fresh on each call.
   * @example
   * ```
   * // exported; invoked by the owning browser through bind:this
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
      targets.push({ id: rootDropId, rect: () => el.getBoundingClientRect() });
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
    const ops = buildReorderOps(typedFolderSiblings(folders, folderDocType, parentId, ctx.compare), from, to);
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
      const ids = typedFolderSiblings(folders, folderDocType, parentId, ctx.compare).map((s) => s.id);
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
    const siblings = typedFolderSiblings(folders, folderDocType, parentId, ctx.compare);
    const index = siblings.findIndex((s) => s.id === f.id);
    const to = ev.key === "ArrowUp" ? index - 1 : ev.key === "ArrowDown" ? index + 1 : -1;
    if (to < 0 || to >= siblings.length) return;
    ev.preventDefault();
    reorderFolder(parentId, index, to);
  }

  // Reactive bridge (mandatory): the tree re-renders on folder doc changes.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const folders = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query(folderDocType);
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

  // A rename starts with the input focused, so keyboard users type the new name directly.
  $effect(() => {
    renameEl?.focus();
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

  /** Toggles the inline rename input for a folder; the rename button and the context menu's
   * Rename item both call this, so the two entry points cannot diverge.
   * @param f - The folder to rename.
   * @example
   * ```
   * // private function; wired to the rename button and the context menu
   * declare const f: WireDocument;
   * toggleRename(f);
   * ```
   */
  function toggleRename(f: WireDocument): void {
    renamingFor = renamingFor === f.id ? null : f.id;
    renameDraft = f.name ?? "";
  }

  /** Toggles a folder's Move-to picker; shared by the move button and the context menu's Move item.
   * @param f - The folder to move.
   * @example
   * ```
   * // private function; wired to the move button and the context menu
   * declare const f: WireDocument;
   * toggleMovePicker(f);
   * ```
   */
  function toggleMovePicker(f: WireDocument): void {
    movePickerFor = movePickerFor === f.id ? null : f.id;
  }

  /** Opens a folder's delete dialog; shared by the delete button and the context menu's Delete item.
   * @param f - The folder to delete.
   * @example
   * ```
   * // private function; wired to the delete button and the context menu
   * declare const f: WireDocument;
   * askDelete(f);
   * ```
   */
  function askDelete(f: WireDocument): void {
    deleteDialogFor = f.id;
  }

  /** The open context menu's position and items, or null when none is open. */
  let contextMenu = $state<ContextMenuRequest | null>(null);
  /** One context-menu trigger per folder id, so a row's hold timer survives re-renders. */
  const rowTriggers = new Map<string, ContextMenuTrigger>();

  /** The context-menu items for a folder: the same three actions the row's buttons expose.
   * @param f - The folder the menu opens for.
   * @returns Rename, Move and Delete, or none when the tree is read-only.
   * @example
   * ```
   * // private function; read by the row's context-menu trigger
   * declare const f: WireDocument;
   * contextItems(f);
   * ```
   */
  function contextItems(f: WireDocument): ContextMenuItem[] {
    if (!mutable) return [];
    return [
      { id: "rename", labelKey: labels.rename, action: () => toggleRename(f) },
      { id: "move", labelKey: labels.move, action: () => toggleMovePicker(f) },
      { id: "delete", labelKey: labels.delete, action: () => askDelete(f) },
    ];
  }

  /** The row's context-menu trigger, created once per folder id. An in-progress folder drag
   * suppresses the menu.
   * @param f - The folder whose row the trigger serves.
   * @returns The trigger's handlers.
   * @example
   * ```
   * // private function; spread onto each folder row
   * declare const f: WireDocument;
   * triggerFor(f);
   * ```
   */
  function triggerFor(f: WireDocument): ContextMenuTrigger {
    let trigger = rowTriggers.get(f.id);
    if (!trigger) {
      trigger = createContextMenuTrigger({
        getItems: () => {
          const current = folders.find((x) => x.id === f.id);
          return current ? contextItems(current) : [];
        },
        onOpen: (request) => (contextMenu = request),
        canOpen: () => folderDrag.dragging === null,
      });
      rowTriggers.set(f.id, trigger);
    }
    return trigger;
  }

  /** Commits the create input as a new folder under the current selection (root when none).
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
    const sort = appendTypedFolderSortKey(folders, folderDocType, selectedFolder);
    ctx.dispatchIntent([
      { op: "create", doc: buildFolder(ctx.world, name, selectedFolder, sort) },
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

  /** Runs the owner's delete with the dialog's chosen button.
   * @param folder - The folder to delete.
   * @param choiceId - The chosen `FolderDeleteChoice` id.
   * @example
   * ```
   * // private function; wired to the delete dialog below
   * declare const folder: WireDocument;
   * void confirmDelete(folder, "reparent");
   * ```
   */
  async function confirmDelete(folder: WireDocument, choiceId: string): Promise<void> {
    deleteDialogFor = null;
    try {
      await onDelete(folder, choiceId);
      if (selectedFolder === folder.id) onSelectFolder(null);
    } catch (e) {
      ctx.notify(t(labels.error, { message: String(e) }));
    }
  }

  /** Handles OS files dropped on a folder node (upload targets). Dragged items and folders
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
    class:drop-into={dropHover?.over === rootDropId}
    data-testid="folder-node-all"
    bind:this={allEl}
    onclick={() => onSelectFolder(null)}
  >{t(labels.all)}</button>

  {#snippet nodes(parent: string | null, depth: number)}
    {#each typedFolderChildren(folders, folderDocType, parent, ctx.compare) as f (f.id)}
      <div
        class="row"
        class:drop-before={folderHover?.over === f.id && folderHover.placement === "before"}
        class:drop-after={folderHover?.over === f.id && folderHover.placement === "after"}
        class:drop-into={(folderHover?.over === f.id && folderHover.placement === "into") || dropHover?.over === f.id}
        style:padding-left="{depth * 0.75}rem"
        use:trackRow={f.id}
        role="presentation"
        onkeydown={(e) => onRowKeydown(f, e)}
        {...triggerFor(f)}
      >
        {#if mutable && ctx.canEdit(f, "/engine/sort")}
          <button
            type="button"
            class="mini drag-handle"
            data-testid={"folder-drag-" + f.id}
            aria-label={t(labels.dragFolder)}
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
            title={t(labels.rename)}
            onclick={() => toggleRename(f)}
          >✎</button>
          <button
            type="button"
            class="mini"
            data-testid={"folder-move-" + f.id}
            title={t(labels.move)}
            onclick={() => toggleMovePicker(f)}
          >⇄</button>
          <button
            type="button"
            class="mini"
            data-testid={"folder-delete-" + f.id}
            title={t(labels.delete)}
            onclick={() => askDelete(f)}
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
        <div class="move-picker" role="group" aria-label={t(labels.move)}>
          <button type="button" data-testid="folder-move-target-root" onclick={() => moveFolder(f.id, null)}>
            {t(labels.moveToRoot)}
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
          aria-label={t(labels.delete)}
          bind:this={deleteDialogEl}
        >
          <p>{t(labels.deletePrompt, { name: f.name ?? "" })}</p>
          {#each deleteChoices as choice (choice.id)}
            <button
              type="button"
              class:danger={choice.danger}
              data-testid={"folder-delete-" + choice.id}
              onclick={() => void confirmDelete(f, choice.id)}
            >
              {t(choice.labelKey)}
            </button>
          {/each}
          <button type="button" onclick={() => (deleteDialogFor = null)}>
            {t(labels.cancel)}
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
      placeholder={t(labels.newFolder)}
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

{#if contextMenu}
  <ContextMenu items={contextMenu.items} x={contextMenu.x} y={contextMenu.y} onClose={() => (contextMenu = null)} />
{/if}

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
    min-height: 2.75rem;
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
    user-select: none;
  }
  .mini {
    min-width: 2.75rem;
    min-height: 2.75rem;
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
      min-height: 2.75rem;
      text-align: left;
    }
  }
  .delete-dialog .danger {
    color: var(--danger);
  }
  input {
    min-height: 2.75rem;
    margin-top: 0.375rem;
  }
</style>
