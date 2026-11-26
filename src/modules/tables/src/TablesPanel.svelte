<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import {
    getAppContext,
    ContextMenu,
    DocFolderTree,
    createContextMenuTrigger,
    type ContextMenuItem,
    type ContextMenuRequest,
    type ContextMenuTrigger,
    type FolderTreeLabels,
    type FolderDeleteChoice,
  } from "@shadowcat/ui-kit";
  import {
    TABLE_DOC_TYPE,
    TABLE_FOLDER_DOC_TYPE,
    buildTableDoc,
    buildTableFolderDoc,
    buildMoveOp,
    createDragController,
    firstChannel,
    type DropResult,
    type WireDocument,
    type WireSearchHit,
    type SubscriptionHandle,
  } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  /** The drop-target id of the tree's all-tables row: a table dropped on it leaves its folder.
   * Never collides with a folder id, which is a UUID. */
  const ROOT_DROP_ID = "all-tables";

  /** The shared tree's labels for table folders. */
  const FOLDER_LABELS: FolderTreeLabels = {
    all: "tables.allTables",
    newFolder: "tables.newFolder",
    dragFolder: "tables.dragFolder",
    rename: "tables.renameFolder",
    move: "tables.moveFolder",
    delete: "tables.deleteFolder",
    moveToRoot: "tables.moveToRoot",
    deletePrompt: "tables.deleteFolderPrompt",
    cancel: "tables.cancel",
    error: "tables.folderError",
  };

  /** Deleting a table folder has one outcome: the server moves its tables up before removing it. */
  const FOLDER_DELETE_CHOICES: FolderDeleteChoice[] = [
    { id: "confirm", labelKey: "tables.deleteFolderConfirm", danger: true },
  ];

  // Reactive read of the optimistic view (mandatory bridge): the list re-renders on create/
  // delete.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const allTables = $derived.by((): WireDocument[] => {
    subscribe();
    return [...ctx.documents.query(TABLE_DOC_TYPE)].sort((a, b) => ctx.compare(a.name ?? "", b.name ?? ""));
  });
  const folders = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query(TABLE_FOLDER_DOC_TYPE);
  });

  /** The folder filtering the list, or `null` = every table. */
  let selectedFolder = $state<string | null>(null);
  /** The shared tree instance, read for its drop targets. */
  let tree = $state<ReturnType<typeof DocFolderTree> | null>(null);
  /** The drop resolution of an in-progress table drag, for the tree's highlight. */
  let dropHover = $state<DropResult | null>(null);
  /** The table whose Move-to picker is open, or null. */
  let movePickerFor = $state<string | null>(null);

  // Live FTS search, same shape as `ActorsPanel`'s: cancel-guarded, server-side `docTypes`
  // filter is the ONE filter (no client-side re-filter).
  let query = $state("");
  let searchHits = $state<WireDocument[]>([]);
  $effect(() => {
    const q = query.trim();
    if (!q) { searchHits = []; return; }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20, docTypes: [TABLE_DOC_TYPE] }, (hits: WireSearchHit[]) => {
        if (cancelled) return;
        searchHits = hits.map((h) => h.document);
      })
      .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; handle?.unsubscribe(); };
  });
  // A search spans every folder; otherwise the selected folder (or every table) lists.
  const visibleTables = $derived(
    query.trim()
      ? searchHits
      : selectedFolder === null
        ? allTables
        : allTables.filter((d) => d.parent_id === selectedFolder),
  );
  /** Every folder by name, the Move-to picker's targets. */
  const folderChoices = $derived([...folders].sort((a, b) => ctx.compare(a.name ?? "", b.name ?? "")));

  const channel = $derived.by((): string | null => {
    subscribe();
    return firstChannel(ctx.documents);
  });

  let name = $state("");

  /** Creates a new table owned by the caller (a fresh, empty weighted table) and opens it.
   * @example
   * ```
   * // private function; wired to the create form's onsubmit
   * create();
   * ```
   */
  function create(): void {
    const trimmed = name.trim();
    if (!trimmed) return;
    const doc = buildTableDoc(
      ctx.world,
      trimmed,
      { draw: { kind: "weighted" }, rows: [], description: "" },
      { owner: ctx.selfId, parentId: selectedFolder },
    );
    ctx.dispatchIntent([{ op: "create", doc }]);
    ctx.openDocument({ docId: doc.id });
    name = "";
  }

  /** Draws one row from `doc` into the current channel, surfacing the server's refusal reason
   * (over-cap count, empty table, cycle, missing asset, rate limit) via `ctx.notify`.
   * @param doc The table to draw from.
   * @example
   * ```
   * // private function; wired to the per-row quick-draw button
   * declare const doc: WireDocument;
   * quickDraw(doc);
   * ```
   */
  function quickDraw(doc: WireDocument): void {
    if (channel === null) return;
    ctx.chat.drawTable({ tableId: doc.id, channel, count: 1 }).catch((e) => {
      ctx.notify(String(e instanceof Error ? e.message : e));
    });
  }

  /** Opens a table's sheet; the open button and the context menu's Open item both call this.
   * @param doc The table to open.
   * @example
   * ```
   * // private function; wired to the open button and the context menu
   * declare const doc: WireDocument;
   * openTable(doc);
   * ```
   */
  function openTable(doc: WireDocument): void {
    ctx.openDocument({ docId: doc.id });
  }

  /** Deletes a table; the delete button and the context menu's Delete item both call this.
   * @param doc The table to delete.
   * @example
   * ```
   * // private function; wired to the delete button and the context menu
   * declare const doc: WireDocument;
   * deleteTable(doc);
   * ```
   */
  function deleteTable(doc: WireDocument): void {
    ctx.dispatchIntent([{ op: "delete", doc }]);
  }

  /** Re-parents a table with the stored parent as the OCC pre-image; the drag drop and the
   * Move-to picker both call this, so the two entry points cannot diverge.
   * @param doc The table to move.
   * @param target The destination `table_folder` id (`null` = the world root).
   * @example
   * ```
   * // private function; wired to the drag drop and the Move-to picker
   * declare const doc: WireDocument;
   * moveTable(doc, null);
   * ```
   */
  function moveTable(doc: WireDocument, target: string | null): void {
    movePickerFor = null;
    const current = doc.parent_id ?? null;
    if (target === current) return;
    ctx.dispatchIntent([buildMoveOp(doc.id, target, current)]);
  }

  /** Toggles a table's Move-to picker; the move button and the context menu's Move item both call this.
   * @param doc The table to move.
   * @example
   * ```
   * // private function; wired to the move button and the context menu
   * declare const doc: WireDocument;
   * toggleMovePicker(doc);
   * ```
   */
  function toggleMovePicker(doc: WireDocument): void {
    movePickerFor = movePickerFor === doc.id ? null : doc.id;
  }

  // Pointer-driven row drag from each row's handle (touch-capable). A release outside every
  // tree target drops nothing.
  const drag = createDragController({
    axis: "vertical",
    getTargets: () => tree?.dropTargets() ?? [],
    overIsContainer: () => true,
    onHover: (result) => (dropHover = result),
    onDrop: (result) => {
      if (!result.over) return;
      const doc = allTables.find((d) => d.id === result.dragged);
      if (doc) moveTable(doc, result.over === ROOT_DROP_ID ? null : result.over);
    },
  });

  /** Deletes a table folder; the server moves the tables it holds up to the folder's parent.
   * @param folder The folder to delete.
   * @example
   * ```
   * // private function; passed to the shared tree as onDelete
   * declare const folder: WireDocument;
   * deleteFolder(folder);
   * ```
   */
  function deleteFolder(folder: WireDocument): void {
    ctx.dispatchIntent([{ op: "delete", doc: folder }]);
  }

  /** Builds the document a create-folder commit dispatches, with the caller as its author.
   * @param world The owning world id.
   * @param name The folder's display name.
   * @param parentId The parent folder, or `null` for the root.
   * @param sort The folder's sibling sort key.
   * @returns The unsaved folder document.
   * @example
   * ```
   * // private function; passed to the shared tree as buildFolder
   * buildFolder("w1", "Loot", null, 1000);
   * ```
   */
  function buildFolder(world: string, name: string, parentId: string | null, sort: number): WireDocument {
    return buildTableFolderDoc(world, name, { owner: ctx.selfId, parentId, sort });
  }

  /** The open context menu's position and items, or null when none is open. */
  let contextMenu = $state<ContextMenuRequest | null>(null);
  /** The table whose row the current gesture started on, read by the shared trigger's `getItems`. */
  let gestureDoc: WireDocument | null = null;

  /** The context-menu items for a table: the actions the row's buttons expose, under the same
   * gates (no channel disables Draw, `canDelete` gates Delete).
   * @param doc The table the menu opens for.
   * @returns Open, Draw, Move when the table's parent may change, and Delete when permitted.
   * @example
   * ```
   * // private function; read by the shared trigger
   * declare const doc: WireDocument;
   * contextItems(doc);
   * ```
   */
  function contextItems(doc: WireDocument): ContextMenuItem[] {
    const items: ContextMenuItem[] = [
      { id: "open", labelKey: "tables.open", action: () => openTable(doc) },
      { id: "draw", labelKey: "tables.draw", action: () => quickDraw(doc), disabled: channel === null },
    ];
    if (ctx.canEdit(doc, "/parent_id")) items.push({ id: "move", labelKey: "tables.moveTable", action: () => toggleMovePicker(doc) });
    if (ctx.canDelete(doc)) items.push({ id: "delete", labelKey: "tables.delete", action: () => deleteTable(doc) });
    return items;
  }

  /** The one trigger every row shares; the row records its table in `gestureDoc` before
   * forwarding a gesture that can open the menu. */
  const trigger: ContextMenuTrigger = createContextMenuTrigger({
    getItems: () => (gestureDoc ? contextItems(gestureDoc) : []),
    onOpen: (request) => (contextMenu = request),
    canOpen: () => drag.dragging === null,
  });
</script>

<section class="tables" data-testid="tables-panel">
  <h3>{t("tables.tab")}</h3>
  <input
    class="search"
    type="search"
    data-testid="tables-search"
    placeholder={t("tables.search")}
    aria-label={t("tables.search")}
    bind:value={query}
  />
  <DocFolderTree
    bind:this={tree}
    folderDocType={TABLE_FOLDER_DOC_TYPE}
    labels={FOLDER_LABELS}
    rootDropId={ROOT_DROP_ID}
    deleteChoices={FOLDER_DELETE_CHOICES}
    onDelete={deleteFolder}
    {buildFolder}
    mutable={ctx.canCreate(TABLE_FOLDER_DOC_TYPE)}
    {selectedFolder}
    onSelectFolder={(id) => (selectedFolder = id)}
    {dropHover}
  />
  {#if visibleTables.length === 0}
    <p class="empty">{t("tables.empty")}</p>
  {:else}
    <ul class="list">
      {#each visibleTables as doc (doc.id)}
        <li
          data-testid="table-row"
          data-table-id={doc.id}
          oncontextmenu={(e) => {
            gestureDoc = doc;
            trigger.oncontextmenu(e);
          }}
          onpointerdown={(e) => {
            gestureDoc = doc;
            trigger.onpointerdown(e);
          }}
          onpointerup={trigger.onpointerup}
          onpointercancel={trigger.onpointercancel}
          onpointermove={trigger.onpointermove}
        >
          {#if ctx.canEdit(doc, "/parent_id")}
            <button
              type="button"
              class="drag-handle"
              data-testid="table-drag"
              data-table-id={doc.id}
              aria-label={t("tables.dragTable")}
              onpointerdown={(e) => drag.beginDrag(doc.id, e)}
            >⠿</button>
          {/if}
          <button type="button" data-testid="table-open" data-table-id={doc.id} onclick={() => openTable(doc)}>
            {doc.name ?? t("sheetTable.title")}
          </button>
          <button
            type="button"
            data-testid="table-quick-draw"
            data-table-id={doc.id}
            disabled={channel === null}
            onclick={() => quickDraw(doc)}
          >{t("tables.draw")}</button>
          {#if ctx.canEdit(doc, "/parent_id")}
            <button
              type="button"
              data-testid="table-move"
              data-table-id={doc.id}
              title={t("tables.moveTable")}
              onclick={() => toggleMovePicker(doc)}
            >⇄</button>
          {/if}
          {#if ctx.canDelete(doc)}
            <button
              type="button"
              data-testid="table-delete"
              data-table-id={doc.id}
              onclick={() => deleteTable(doc)}
            >{t("tables.delete")}</button>
          {/if}
          {#if movePickerFor === doc.id}
            <div class="move-picker" role="menu">
              <button type="button" data-testid="table-move-target-root" onclick={() => moveTable(doc, null)}>
                {t("tables.moveToRoot")}
              </button>
              {#each folderChoices as target (target.id)}
                <button
                  type="button"
                  data-testid={"table-move-target-" + target.id}
                  onclick={() => moveTable(doc, target.id)}
                >{target.name}</button>
              {/each}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if ctx.canCreate(TABLE_DOC_TYPE)}
    <form onsubmit={(e) => { e.preventDefault(); create(); }}>
      <input
        data-testid="tables-name"
        placeholder={t("tables.name")}
        aria-label={t("tables.name")}
        bind:value={name}
      />
      <button type="submit" data-testid="tables-create" disabled={!name.trim()}>{t("tables.newTable")}</button>
    </form>
  {/if}
</section>

{#if contextMenu}
  <ContextMenu items={contextMenu.items} x={contextMenu.x} y={contextMenu.y} onClose={() => (contextMenu = null)} />
{/if}

<style lang="scss">
  .tables { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); }
  .search { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); }
  .search:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .empty { opacity: 0.7; font-style: italic; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .list li { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-1); }
  .drag-handle { min-width: 44px; user-select: none; }
  .move-picker { display: flex; flex-direction: column; gap: var(--space-1); flex-basis: 100%; }
  .list button { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); cursor: pointer; }
  form { display: flex; gap: var(--space-1); }
  form input, form button { min-height: 44px; }
</style>
