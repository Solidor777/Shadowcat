<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { TABLE_DOC_TYPE, type TableEntry, type WireSearchHit } from "@shadowcat/core";

  // One result entry inside a `RowEditor` row. Purely presentational + local picker state —
  // every field edit builds the WHOLE replacement `TableEntry` and hands it to `onChange`;
  // the row (and ultimately the sheet) owns the actual `setField` write.
  let {
    entry,
    onChange,
    onRemove,
    disabled = false,
  }: {
    /** The entry this editor renders. */
    entry: TableEntry;
    /** Called with the replacement entry on every field edit. */
    onChange: (next: TableEntry) => void;
    /** Removes this entry from the row. */
    onRemove: () => void;
    /** Disables every control. */
    disabled?: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let docQuery = $state("");
  let docHits = $state<WireSearchHit[]>([]);
  let tableQuery = $state("");
  let tableHits = $state<WireSearchHit[]>([]);

  // Live document picker for a "doc" entry — mirrors the composer's `@doc` picker
  // (`ctx.searchDocuments`, torn down/recreated per query, cancel-guarded against a stale
  // callback firing after a newer query's subscription is already active).
  $effect(() => {
    if (entry.kind !== "doc") { docHits = []; return; }
    const q = docQuery.trim();
    if (!q) { docHits = []; return; }
    let cancelled = false;
    void ctx.searchDocuments(q, { limit: 20 }, (hits) => { if (!cancelled) docHits = hits; })
      .then((h) => { if (cancelled) h.unsubscribe(); })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; };
  });

  // Live table picker for a "draw" entry, narrowed server-side to table documents.
  $effect(() => {
    if (entry.kind !== "draw") { tableHits = []; return; }
    const q = tableQuery.trim();
    if (!q) { tableHits = []; return; }
    let cancelled = false;
    void ctx.searchDocuments(q, { limit: 20, docTypes: [TABLE_DOC_TYPE] }, (hits) => {
      if (!cancelled) tableHits = hits;
    })
      .then((h) => { if (cancelled) h.unsubscribe(); })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; };
  });

  /** Opens the asset browser in image-pick mode and applies the result to an "image" entry.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "Pick image" button
   * pickImage();
   * ```
   */
  function pickImage(): void {
    if (entry.kind !== "image") return;
    void ctx.pickAsset({ kind: "image" }).then((id) => {
      if (id) onChange({ ...entry, asset_id: id });
    });
  }
</script>

<div class="entry">
  <label>{t("sheetTable.entryKind")}
    <select data-testid="entry-kind" aria-label={t("sheetTable.entryKind")} value={entry.kind} {disabled}
      onchange={(e) => {
        const kind = (e.currentTarget as HTMLSelectElement).value as TableEntry["kind"];
        if (kind === "text") onChange({ kind: "text", text: "" });
        else if (kind === "doc") onChange({ kind: "doc", target: { kind: "doc", doc_id: "", embedded_path: null }, label: "" });
        else if (kind === "image") onChange({ kind: "image", asset_id: "", alt: "" });
        else onChange({ kind: "draw", table_id: "", count: 1 });
      }}>
      <option value="text">{t("sheetTable.kindText")}</option>
      <option value="doc">{t("sheetTable.kindDoc")}</option>
      <option value="image">{t("sheetTable.kindImage")}</option>
      <option value="draw">{t("sheetTable.kindDraw")}</option>
    </select></label>

  {#if entry.kind === "text"}
    <textarea data-testid="entry-text" aria-label={t("sheetTable.kindText")} {disabled}
      value={entry.text} onchange={(e) => onChange({ kind: "text", text: (e.currentTarget as HTMLTextAreaElement).value })}></textarea>
  {:else if entry.kind === "doc"}
    <label>{t("sheetTable.label")}
      <input data-testid="entry-label" aria-label={t("sheetTable.label")} {disabled}
        value={entry.label} onchange={(e) => onChange({ ...entry, label: (e.currentTarget as HTMLInputElement).value })} /></label>
    <input data-testid="entry-pick-doc" placeholder={t("sheetTable.pickDoc")} aria-label={t("sheetTable.pickDoc")} {disabled}
      bind:value={docQuery} />
    {#if docHits.length > 0}
      <ul class="hits">
        {#each docHits as hit (hit.document.id)}
          <li><button type="button" {disabled} onclick={() => { onChange({ kind: "doc", target: { kind: "doc", doc_id: hit.document.id, embedded_path: null }, label: entry.label || (hit.document.name ?? "") }); docQuery = ""; }}>{hit.document.name ?? hit.document.id}</button></li>
        {/each}
      </ul>
    {/if}
  {:else if entry.kind === "image"}
    <button type="button" data-testid="entry-pick-image" {disabled} onclick={pickImage}>{t("sheetTable.pickImage")}</button>
    <label>{t("sheetTable.alt")}
      <input data-testid="entry-alt" aria-label={t("sheetTable.alt")} {disabled}
        value={entry.alt} onchange={(e) => onChange({ ...entry, alt: (e.currentTarget as HTMLInputElement).value })} /></label>
  {:else}
    <input data-testid="entry-pick-table" placeholder={t("sheetTable.pickTable")} aria-label={t("sheetTable.pickTable")} {disabled}
      bind:value={tableQuery} />
    {#if tableHits.length > 0}
      <ul class="hits">
        {#each tableHits as hit (hit.document.id)}
          <li><button type="button" {disabled} onclick={() => { onChange({ ...entry, table_id: hit.document.id }); tableQuery = ""; }}>{hit.document.name ?? hit.document.id}</button></li>
        {/each}
      </ul>
    {/if}
    <label>{t("sheetTable.count")}
      <input data-testid="entry-count" type="number" min="1" aria-label={t("sheetTable.count")} {disabled}
        value={entry.count} onchange={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          if (Number.isNaN(v)) return;
          onChange({ ...entry, count: v });
        }} /></label>
  {/if}

  <button type="button" data-testid="entry-remove" {disabled} onclick={onRemove}>{t("sheetTable.removeEntry")}</button>
</div>

<style lang="scss">
  .entry { display: flex; flex-direction: column; gap: 4px; padding: var(--space-1); border: 1px solid var(--border); border-radius: var(--radius-1); }
  .hits { list-style: none; margin: 0; padding: 0; }
  .hits button { min-height: 32px; text-align: left; }
</style>
