<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildMoveOp, type WireDocument } from "@shadowcat/core";
  import type { NoteTreeNode } from "./tree";

  // Recursive tree rendering (Move-to/Delete/expand-collapse per row); also used, with
  // every node forced to a leaf, to render a flat search-hit list from the panel — the ONE
  // row markup, so a hit and a tree node never drift into two shapes.
  let {
    nodes,
    allNotes,
  }: {
    /** The nodes to render at this level (their `children` recurse). */
    nodes: NoteTreeNode[];
    /** Every note in the caller's current view — the Move-to target list (every note but the
     * one being moved, plus "root"). */
    allNotes: WireDocument[];
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let expanded = $state<Set<string>>(new Set());
  let moveOpenFor = $state<string | null>(null);

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

{#snippet row(node: NoteTreeNode, depth: number)}
  <li data-testid="note-row" data-note-id={node.doc.id} style:padding-left="{depth * 0.75}rem">
    <div class="row">
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
        onclick={() => ctx.openDocument({ docId: node.doc.id })}
      >{node.doc.name ?? t("sheetNote.untitled")}</button>
      {#if ctx.canDelete(node.doc)}
        <button
          type="button"
          data-testid="note-delete"
          data-note-id={node.doc.id}
          onclick={() => ctx.dispatchIntent([{ op: "delete", doc: node.doc }])}
        >{t("notes.delete")}</button>
      {/if}
      {#if ctx.role === "gm"}
        <button
          type="button"
          data-testid="note-move"
          data-note-id={node.doc.id}
          onclick={() => (moveOpenFor = moveOpenFor === node.doc.id ? null : node.doc.id)}
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
          {@render row(child, depth + 1)}
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<ul class="tree">
  {#each nodes as node (node.doc.id)}
    {@render row(node, 0)}
  {/each}
</ul>

<style lang="scss">
  .tree { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--space-1); }
  .row button { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); cursor: pointer; }
</style>
