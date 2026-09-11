<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { TABLE_DOC_TYPE, buildTableDoc, firstChannel, type WireDocument, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the optimistic view (mandatory bridge): the list re-renders on create/
  // delete.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const allTables = $derived.by((): WireDocument[] => {
    subscribe();
    return [...ctx.documents.query(TABLE_DOC_TYPE)].sort((a, b) => (a.name ?? "").localeCompare(b.name ?? ""));
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
        <li data-testid="table-row" data-table-id={doc.id}>
          <button type="button" data-testid="table-open" data-table-id={doc.id} onclick={() => ctx.openDocument({ docId: doc.id })}>
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
              onclick={() => ctx.dispatchIntent([{ op: "delete", doc }])}
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
