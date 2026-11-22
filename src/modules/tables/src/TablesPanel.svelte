<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import {
    getAppContext,
    ContextMenu,
    createContextMenuTrigger,
    type ContextMenuItem,
    type ContextMenuRequest,
    type ContextMenuTrigger,
  } from "@shadowcat/ui-kit";
  import { TABLE_DOC_TYPE, buildTableDoc, firstChannel, type WireDocument, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the optimistic view (mandatory bridge): the list re-renders on create/
  // delete.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const allTables = $derived.by((): WireDocument[] => {
    subscribe();
    return [...ctx.documents.query(TABLE_DOC_TYPE)].sort((a, b) => ctx.compare(a.name ?? "", b.name ?? ""));
  });

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
  const visibleTables = $derived(query.trim() ? searchHits : allTables);

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
    const doc = buildTableDoc(ctx.world, trimmed, { draw: { kind: "weighted" }, rows: [], description: "" }, { owner: ctx.selfId });
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

  /** The open context menu's position and items, or null when none is open. */
  let contextMenu = $state<ContextMenuRequest | null>(null);
  /** The table whose row the current gesture started on, read by the shared trigger's `getItems`. */
  let gestureDoc: WireDocument | null = null;

  /** The context-menu items for a table: the actions the row's buttons expose, under the same
   * gates (no channel disables Draw, `canDelete` gates Delete).
   * @param doc The table the menu opens for.
   * @returns Open, Draw, and Delete when permitted.
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
    if (ctx.canDelete(doc)) items.push({ id: "delete", labelKey: "tables.delete", action: () => deleteTable(doc) });
    return items;
  }

  /** The one trigger every row shares; the row records its table in `gestureDoc` before
   * forwarding a gesture that can open the menu. */
  const trigger: ContextMenuTrigger = createContextMenuTrigger({
    getItems: () => (gestureDoc ? contextItems(gestureDoc) : []),
    onOpen: (request) => (contextMenu = request),
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
          {#if ctx.canDelete(doc)}
            <button
              type="button"
              data-testid="table-delete"
              data-table-id={doc.id}
              onclick={() => deleteTable(doc)}
            >{t("tables.delete")}</button>
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
  .list li { display: flex; align-items: center; gap: var(--space-1); }
  .list button { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); cursor: pointer; }
  form { display: flex; gap: var(--space-1); }
  form input, form button { min-height: 44px; }
</style>
