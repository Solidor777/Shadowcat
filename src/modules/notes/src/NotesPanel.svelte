<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { NOTE_DOC_TYPE, buildNoteDoc, type WireDocument, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";
  import NoteTree from "./NoteTree.svelte";
  import { buildNoteTree, type NoteTreeNode } from "./tree";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the optimistic view (mandatory bridge): the tree re-renders on create/
  // delete/move.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const allNotes = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query(NOTE_DOC_TYPE);
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
      .searchDocuments(q, { limit: 20, docTypes: [NOTE_DOC_TYPE] }, (hits: WireSearchHit[]) => {
        if (cancelled) return;
        searchHits = hits.map((h) => h.document);
      })
      .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; handle?.unsubscribe(); };
  });

  // A non-empty query replaces the tree with a flat hit list — every hit rendered as a
  // childless node through the SAME `NoteTree` row markup as the real tree, so a search result
  // never forks the row's Delete/Move-to/open affordances into a second shape.
  const tree = $derived.by((): NoteTreeNode[] =>
    query.trim() ? searchHits.map((doc): NoteTreeNode => ({ doc, children: [] })) : buildNoteTree(allNotes),
  );

  let name = $state("");

  /** Creates a private note owned by the caller and opens it.
   * @example
   * ```
   * // private function; wired to the create form's onsubmit
   * create();
   * ```
   */
  function create(): void {
    const trimmed = name.trim();
    if (!trimmed) return;
    const doc = buildNoteDoc(ctx.world, trimmed, "", { owner: ctx.selfId });
    ctx.dispatchIntent([{ op: "create", doc }]);
    ctx.openDocument({ docId: doc.id });
    name = "";
  }
</script>

<section class="notes" data-testid="notes-panel">
  <h3>{t("notes.tab")}</h3>
  <input
    class="search"
    type="search"
    data-testid="notes-search"
    placeholder={t("notes.search")}
    aria-label={t("notes.search")}
    bind:value={query}
  />
  {#if tree.length === 0}
    <p class="empty">{t("notes.empty")}</p>
  {:else}
    <NoteTree nodes={tree} allNotes={allNotes} />
  {/if}
  {#if ctx.canCreate(NOTE_DOC_TYPE)}
    <form onsubmit={(e) => { e.preventDefault(); create(); }}>
      <input
        data-testid="notes-name"
        placeholder={t("notes.name")}
        aria-label={t("notes.name")}
        bind:value={name}
      />
      <button type="submit" data-testid="notes-create" disabled={!name.trim()}>{t("notes.newNote")}</button>
    </form>
  {/if}
</section>

<style lang="scss">
  .notes { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); }
  .search { min-height: 44px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: var(--text-primary); }
  .search:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .empty { opacity: 0.7; font-style: italic; }
  form { display: flex; gap: var(--space-1); }
  form input, form button { min-height: 44px; }
</style>
