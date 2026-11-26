<script lang="ts">
  import {
    getAppContext,
    ContextMenu,
    createContextMenuTrigger,
    type ContextMenuItem,
    type ContextMenuRequest,
    type ContextMenuTrigger,
  } from "@shadowcat/ui-kit";
  import { buildMoveOp, buildReorderOps, createDragController, dropIndex, type DropResult, type WireDocument } from "@shadowcat/core";
  import { noteSiblings, type NoteTreeNode } from "./tree";

  // Recursive tree rendering (Move-to/Delete/expand-collapse per row); also used, with
  // every node forced to a leaf, to render a flat search-hit list from the panel — the ONE
  // row markup, so a hit and a tree node never drift into two shapes.
  let {
    nodes,
    allNotes,
    reorderable = false,
  }: {
    /** The nodes to render at this level (their `children` recurse). */
    nodes: NoteTreeNode[];
    /** Every note in the caller's current view — the Move-to target list (every note but the
     * one being moved, plus "root"). */
    allNotes: WireDocument[];
    /** Whether rows can be reordered among their siblings (drag handle + Alt+Arrow). Off for a
     * flat search-hit list, whose rows are not siblings. */
    reorderable?: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let expanded = $state<Set<string>>(new Set());
  let moveOpenFor = $state<string | null>(null);

  /** Each row's element by note id: the sibling-reorder drag's geometry lookup. */
  const rowEls = new Map<string, HTMLElement>();
  /** The sibling list the in-progress drag reorders within. */
  let dragLevel: NoteTreeNode[] = [];
  /** The drop resolution under the pointer during a drag, for the drop-indicator styling. */
  let hover = $state<DropResult | null>(null);

  /** `use:` action registering a row element under its note id.
   * @param node The row's element.
   * @param id The note's document id.
   * @returns A teardown removing the registration.
   * @example
   * ```svelte
   * <div use:trackRow={"note-id"}></div>
   * ```
   */
  function trackRow(node: HTMLElement, id: string): {
    /** Removes the registration on unmount. */
    destroy: () => void;
  } {
    rowEls.set(id, node);
    return { destroy: () => rowEls.delete(id) };
  }

  /** The ONE reorder dispatch both the pointer drop and the Alt+Arrow keys call, so the two
   * input methods can never produce different operations for the same logical move.
   * @param level The nodes at the dragged note's level, in display order.
   * @param from The moved note's index.
   * @param to Its destination index (post-removal convention).
   * @example
   * ```
   * // private function; invoked from the drag controller's onDrop and onRowKeydown
   * declare const level: NoteTreeNode[];
   * reorder(level, 0, 1);
   * ```
   */
  function reorder(level: NoteTreeNode[], from: number, to: number): void {
    const ops = buildReorderOps(noteSiblings(level), from, to);
    if (ops) ctx.dispatchIntent(ops);
  }

  const drag = createDragController({
    axis: "vertical",
    snapToNearest: true,
    getTargets: () => dragLevel.map((n) => ({ id: n.doc.id, rect: () => rowEls.get(n.doc.id)?.getBoundingClientRect() ?? new DOMRect() })),
    onHover: (result) => (hover = result),
    onDrop: (result) => {
      if (!result.over || result.placement === "into") return;
      const ids = dragLevel.map((n) => n.doc.id);
      const to = dropIndex(ids, result.dragged, result.over, result.placement);
      if (to !== null) reorder(dragLevel, ids.indexOf(result.dragged), to);
    },
  });

  /** Whether a row's drag handle and keyboard reorder are available.
   * @param doc The note document.
   * @param level The nodes at the note's level. A renumber after an exhausted sort gap rewrites
   * siblings' keys in one batch the server rejects whole, so the viewer must be able to write
   * every sibling's sort key, not only this note's.
   * @returns `true` when reordering is on and the viewer may write the sort key of every sibling.
   * @example
   * ```
   * // private function; gates the drag handle in the row markup
   * declare const doc: WireDocument, level: NoteTreeNode[];
   * canReorder(doc, level);
   * ```
   */
  function canReorder(doc: WireDocument, level: NoteTreeNode[]): boolean {
    return reorderable && ctx.canEdit(doc, "/engine/sort") && level.every((n) => ctx.canEdit(n.doc, "/engine/sort"));
  }

  /** Alt+ArrowUp/Down moves the focused row one place among its siblings.
   * @param level The nodes at the row's level.
   * @param doc The row's note document.
   * @param ev The keydown event.
   * @example
   * ```
   * // private function; wired to each row's onkeydown
   * declare const level: NoteTreeNode[], doc: WireDocument, ev: KeyboardEvent;
   * onRowKeydown(level, doc, ev);
   * ```
   */
  function onRowKeydown(level: NoteTreeNode[], doc: WireDocument, ev: KeyboardEvent): void {
    if (!ev.altKey || !canReorder(doc, level)) return;
    const index = level.findIndex((n) => n.doc.id === doc.id);
    const to = ev.key === "ArrowUp" ? index - 1 : ev.key === "ArrowDown" ? index + 1 : -1;
    if (to < 0 || to >= level.length) return;
    ev.preventDefault();
    reorder(level, index, to);
  }

  /** Starts a pointer drag of a row among its siblings.
   * @param level The nodes at the row's level.
   * @param doc The row's note document.
   * @param ev The `pointerdown` event on the drag handle.
   * @example
   * ```
   * // private function; wired to each drag handle's onpointerdown
   * declare const level: NoteTreeNode[], doc: WireDocument, ev: PointerEvent;
   * onDragStart(level, doc, ev);
   * ```
   */
  function onDragStart(level: NoteTreeNode[], doc: WireDocument, ev: PointerEvent): void {
    dragLevel = level;
    drag.beginDrag(doc.id, ev);
  }

  /** Toggles a node's children visibility.
   * @param id The node's document id.
   * @example
   * ```
   * // private function; wired to the per-node expand/collapse button
   * toggle("n1");
   * ```
   */
  function toggle(id: string): void {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id); else next.add(id);
    expanded = next;
  }

  /** Opens a note's sheet; the open button and the context menu's Open item both call this.
   * @param doc The note to open.
   * @example
   * ```
   * // private function; wired to the open button and the context menu
   * declare const doc: WireDocument;
   * openNote(doc);
   * ```
   */
  function openNote(doc: WireDocument): void {
    ctx.openDocument({ docId: doc.id });
  }

  /** Toggles a note's Move-to picker; the move button and the context menu's Move item both call this.
   * @param doc The note to move.
   * @example
   * ```
   * // private function; wired to the move button and the context menu
   * declare const doc: WireDocument;
   * toggleMove(doc);
   * ```
   */
  function toggleMove(doc: WireDocument): void {
    moveOpenFor = moveOpenFor === doc.id ? null : doc.id;
  }

  /** Deletes a note; the delete button and the context menu's Delete item both call this.
   * @param doc The note to delete.
   * @example
   * ```
   * // private function; wired to the delete button and the context menu
   * declare const doc: WireDocument;
   * deleteNote(doc);
   * ```
   */
  function deleteNote(doc: WireDocument): void {
    ctx.dispatchIntent([{ op: "delete", doc }]);
  }

  /** The open context menu's position and items, or null when none is open. */
  let contextMenu = $state<ContextMenuRequest | null>(null);
  /** The note whose row the current gesture started on, read by the shared trigger's `getItems`. */
  let gestureDoc: WireDocument | null = null;

  /** The context-menu items for a note: the actions the row's buttons expose, under the same
   * gates (`canDelete` for Delete, the GM role for Move to).
   * @param doc The note the menu opens for.
   * @returns Open, then Move to and Delete when permitted.
   * @example
   * ```
   * // private function; read by the row's context-menu trigger
   * declare const doc: WireDocument;
   * contextItems(doc);
   * ```
   */
  function contextItems(doc: WireDocument): ContextMenuItem[] {
    const items: ContextMenuItem[] = [{ id: "open", labelKey: "notes.open", action: () => openNote(doc) }];
    if (ctx.role === "gm") items.push({ id: "move", labelKey: "notes.moveTo", action: () => toggleMove(doc) });
    if (ctx.canDelete(doc)) items.push({ id: "delete", labelKey: "notes.delete", action: () => deleteNote(doc) });
    return items;
  }

  /** The one trigger every row shares; an in-progress row drag suppresses the menu. The row
   * records its note in `gestureDoc` before forwarding a gesture that can open the menu. */
  const trigger: ContextMenuTrigger = createContextMenuTrigger({
    getItems: () => (gestureDoc ? contextItems(gestureDoc) : []),
    onOpen: (request) => (contextMenu = request),
    canOpen: () => drag.dragging === null,
  });

  /** Dispatches the Move-to op for `doc`, with the stored parent as the OCC pre-image.
   * @param doc The note being moved.
   * @param target The new parent, or `null` for root.
   * @example
   * ```
   * // private function; wired to the Move-to select's onchange
   * declare const doc: WireDocument;
   * move(doc, null);
   * ```
   */
  function move(doc: WireDocument, target: string | null): void {
    moveOpenFor = null;
    ctx.dispatchIntent([buildMoveOp(doc.id, target, doc.parent_id ?? null)]);
  }
</script>

{#snippet row(node: NoteTreeNode, depth: number, level: NoteTreeNode[])}
  <li data-testid="note-row" data-note-id={node.doc.id} style:padding-left="{depth * 0.75}rem">
    <div
      class="row"
      class:drop-before={hover?.over === node.doc.id && hover.placement === "before"}
      class:drop-after={hover?.over === node.doc.id && hover.placement === "after"}
      use:trackRow={node.doc.id}
      role="presentation"
      onkeydown={(e) => onRowKeydown(level, node.doc, e)}
      oncontextmenu={(e) => {
        gestureDoc = node.doc;
        trigger.oncontextmenu(e);
      }}
      onpointerdown={(e) => {
        gestureDoc = node.doc;
        trigger.onpointerdown(e);
      }}
      onpointerup={trigger.onpointerup}
      onpointercancel={trigger.onpointercancel}
      onpointermove={trigger.onpointermove}
    >
      {#if canReorder(node.doc, level)}
        <button
          type="button"
          class="drag-handle"
          data-testid="note-drag-handle"
          data-note-id={node.doc.id}
          aria-label={t("notes.dragHandle")}
          onpointerdown={(e) => onDragStart(level, node.doc, e)}
        >⠿</button>
      {/if}
      {#if node.children.length > 0}
        <button
          type="button"
          data-testid="note-toggle"
          data-note-id={node.doc.id}
          aria-label={expanded.has(node.doc.id) ? t("notes.collapse") : t("notes.expand")}
          onclick={() => toggle(node.doc.id)}
        >{expanded.has(node.doc.id) ? "▾" : "▸"}</button>
      {/if}
      <button
        type="button"
        data-testid="note-open"
        data-note-id={node.doc.id}
        onclick={() => openNote(node.doc)}
      >{node.doc.name ?? t("sheetNote.untitled")}</button>
      {#if ctx.canDelete(node.doc)}
        <button
          type="button"
          data-testid="note-delete"
          data-note-id={node.doc.id}
          onclick={() => deleteNote(node.doc)}
        >{t("notes.delete")}</button>
      {/if}
      {#if ctx.role === "gm"}
        <button
          type="button"
          data-testid="note-move"
          data-note-id={node.doc.id}
          onclick={() => toggleMove(node.doc)}
        >{t("notes.moveTo")}</button>
      {/if}
    </div>
    {#if moveOpenFor === node.doc.id}
      <select
        data-testid="note-move-target"
        data-note-id={node.doc.id}
        aria-label={t("notes.moveTo")}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLSelectElement).value;
          move(node.doc, v === "" ? null : v);
        }}
      >
        <option value="">{t("notes.root")}</option>
        {#each allNotes.filter((n) => n.id !== node.doc.id) as target (target.id)}
          <option value={target.id}>{target.name ?? t("sheetNote.untitled")}</option>
        {/each}
      </select>
    {/if}
    {#if node.children.length > 0 && expanded.has(node.doc.id)}
      <ul>
        {#each node.children as child (child.doc.id)}
          {@render row(child, depth + 1, node.children)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<ul class="tree">
  {#each nodes as node (node.doc.id)}
    {@render row(node, 0, nodes)}
  {/each}
</ul>

{#if contextMenu}
  <ContextMenu items={contextMenu.items} x={contextMenu.x} y={contextMenu.y} onClose={() => (contextMenu = null)} />
{/if}

<style lang="scss">
  .tree { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--space-1); }
  .row.drop-before { box-shadow: 0 -2px 0 var(--accent); }
  .row.drop-after { box-shadow: 0 2px 0 var(--accent); }
  .drag-handle { min-width: 44px; user-select: none; }
  .row button { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); cursor: pointer; }
</style>
