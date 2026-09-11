<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, SegmentList } from "@shadowcat/ui-kit";
  import { getPointer, firstChannel, parseNoteBody, buildNoteDoc, NOTE_DOC_TYPE, type WireDocument } from "@shadowcat/core";

  // Note sheet: title + visibility over the envelope/permissions bands, the rendered body
  // (server-derived `engine.body`, via `SegmentList`), a draft-base edit flow over the
  // author's raw `engine.source`, sort, and parent/children tree navigation. Reads the
  // OPTIMISTIC store; every write's `old` is the RAW current stored value.
  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The note document this sheet edits (a note is never embedded — always top-level). */
    docId: string;
    /** The write root for the opaque `system` tree; `basePrefix`/`enginePrefix`/`namePrefix`
     * below are derived from it, same pattern as every other sheet. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to the header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  /** One entry in the children list — display-only, derived from a query hit. */
  type NoteChildSummary = {
    /** The child note's document id. */
    id: string;
    /** The child note's display name (`t("sheetNote.untitled")` when unnamed). */
    name: string;
  };
  /** Structural view of a note's `engine` needed only to read `sort` for ordering
   * `children` — an `engine` is `unknown` until narrowed, so this stands in for the full
   * `NoteEngine` type at that one read site. */
  type SortableEngine = {
    /** `NoteEngine.sort`, read as `number` (see `sort`'s own doc for the wire-shape gap). */
    sort?: number;
  };

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const enginePrefix = $derived(`${basePrefix}/engine`);
  const namePrefix = $derived(`${basePrefix}/name`);
  const sourcePath = $derived(`${enginePrefix}/source`);
  const sortPath = $derived(`${enginePrefix}/sort`);

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const name = $derived.by((): string | null => (doc ? (getPointer(doc, namePrefix) as string | null | undefined) ?? null : null));
  /** The stored `NoteEngine.source` — over the wire this is a plain string; reading it via
   * `getPointer` needs no cast since `source` is a string field, unlike `sort` below. */
  const storedSource = $derived.by((): string => (doc ? ((getPointer(doc, sourcePath) as string | undefined) ?? "") : ""));
  /** `NoteEngine.sort`'s ts-rs mapping is `bigint` (an i64), but the wire value is a plain JSON
   * number (`serde_json` never emits a bigint) — same gap `buildNoteDoc` documents. Read as
   * `number` here, matching what is actually on the wire. */
  const sort = $derived.by((): number => (doc ? ((getPointer(doc, sortPath) as number | undefined) ?? 0) : 0));
  const channel = $derived.by((): string | null => {
    subscribe();
    return firstChannel(ctx.documents);
  });
  const body = $derived.by(() => (doc ? parseNoteBody(doc) : null));

  const canEditVisibility = $derived(!!doc && ctx.canEdit(doc, "/permissions/default"));
  const canEditSource = $derived(!!doc && ctx.canEdit(doc, sourcePath));

  /** The children of this note (`parent_id === docId`), ordered by `(engine.sort, created_at)`. */
  const children = $derived.by((): NoteChildSummary[] => {
    subscribe();
    return ctx.documents
      .query(NOTE_DOC_TYPE)
      .filter((d) => d.parent_id === docId)
      .sort((a, b) => {
        const sa = (a.engine as SortableEngine | null | undefined)?.sort ?? 0;
        const sb = (b.engine as SortableEngine | null | undefined)?.sort ?? 0;
        return sa - sb || a.created_at - b.created_at;
      })
      .map((d) => ({ id: d.id, name: d.name ?? t("sheetNote.untitled") }));
  });

  /** The parent note's display name, when `doc.parent_id` resolves in the store — drives the
   * "Up to <parent>" link. `undefined` when there is no parent, or it does not (yet) resolve. */
  const parentName = $derived.by((): string | undefined => {
    subscribe();
    if (!doc?.parent_id) return undefined;
    return ctx.documents.get(doc.parent_id)?.name ?? undefined;
  });

  // Draft-base edit flow: the base — not the live stored value at save time — is the OCC
  // pre-image on purpose; a concurrent edit refuses with `conflict`, surfaced by the shell's
  // reject toast, and the draft is retained for the author to reconcile by hand.
  let draft = $state<string | null>(null);
  let draftBase = $state("");

  /** Opens the editor, seeding both the draft and its OCC base from the current stored source.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Edit button
   * startEdit();
   * ```
   */
  function startEdit(): void {
    draftBase = storedSource;
    draft = storedSource;
  }

  /** Discards the draft without writing. Re-opens from the CURRENT stored value (never the
   * stale draft), matching spec's "Reload" affordance.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Reload button
   * reload();
   * ```
   */
  function reload(): void {
    draftBase = storedSource;
    draft = storedSource;
  }

  /** Commits the draft: ONE `setField` write carrying `draftBase` (not the live stored value)
   * as the OCC pre-image, then closes the editor.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Save button
   * save();
   * ```
   */
  function save(): void {
    if (draft === null || !doc) return;
    setField(ctx, docId, sourcePath, draftBase, draft);
    draft = null;
  }

  /** Discards the draft without writing.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Cancel button
   * cancelEdit();
   * ```
   */
  function cancelEdit(): void {
    draft = null;
  }

  /** Update the envelope `name` field.
   * @param value The new title.
   * @example
   * ```
   * // private function; not part of the public API — wired to the title input
   * setName("Session 1 recap");
   * ```
   */
  function setName(value: string): void {
    if (!doc) return;
    setField(ctx, docId, namePrefix, name, value);
  }

  /** Update the note's whole-document visibility (`/permissions/default`); the author's own
   * `Owner` grant is untouched, so sharing never demotes the author.
   * @param value The new default visibility (`"none"` or `"observer"`).
   * @example
   * ```
   * // private function; not part of the public API — wired to the visibility select
   * setVisibility("observer");
   * ```
   */
  function setVisibility(value: "none" | "observer"): void {
    if (!doc) return;
    setField(ctx, docId, "/permissions/default", doc.permissions.default, value);
  }

  /** Update `engine/sort`.
   * @param value The new sort key.
   * @example
   * ```
   * // private function; not part of the public API — wired to the sort input
   * setSort(3);
   * ```
   */
  function setSort(value: number): void {
    if (!doc) return;
    setField(ctx, docId, sortPath, sort, value);
  }

  /** Creates a new child note under this one and opens it.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "New child note" button
   * newChild();
   * ```
   */
  function newChild(): void {
    const child = buildNoteDoc(ctx.world, t("sheetNote.untitled"), "", { parentId: docId, owner: ctx.selfId });
    ctx.dispatchIntent([{ op: "create", doc: child }]);
    ctx.openDocument({ docId: child.id });
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{name ?? t("sheetNote.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc}
    <label>{t("sheetNote.title")}
      <input data-testid="note-title" aria-label={t("sheetNote.title")} value={name ?? ""}
        onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>

    {#if canEditVisibility}
      <label>{t("sheetNote.visibility")}
        <select data-testid="note-visibility" aria-label={t("sheetNote.visibility")} value={doc.permissions.default}
          onchange={(e) => setVisibility((e.currentTarget as HTMLSelectElement).value === "observer" ? "observer" : "none")}>
          <option value="none">{t("sheetNote.visibilityPrivate")}</option>
          <option value="observer">{t("sheetNote.visibilityShared")}</option>
        </select></label>
    {/if}

    <fieldset class="body" disabled={(channel ?? "") === ""} data-testid="note-body">
      <legend>{t("sheetNote.title")}</legend>
      {#if body}
        <SegmentList segments={body} channel={channel ?? ""} />
      {:else}
        <p class="unrenderable">{t("sheetNote.unrenderable")}</p>
      {/if}
    </fieldset>

    {#if canEditSource}
      {#if draft === null}
        <button type="button" data-testid="note-edit" onclick={startEdit}>{t("sheetNote.edit")}</button>
      {:else}
        {#if storedSource !== draftBase}
          <p class="changed-remotely">{t("sheetNote.changedRemotely")}</p>
        {/if}
        <textarea data-testid="note-source" aria-label={t("sheetNote.edit")} bind:value={draft}></textarea>
        <div class="editor-actions">
          <button type="button" data-testid="note-save" onclick={save}>{t("sheetNote.save")}</button>
          <button type="button" data-testid="note-cancel" onclick={cancelEdit}>{t("sheetNote.cancel")}</button>
          <button type="button" onclick={reload}>{t("sheetNote.reload")}</button>
        </div>
      {/if}
    {/if}

    <label>{t("sheetNote.sort")}
      <input data-testid="note-sort" type="number" aria-label={t("sheetNote.sort")} value={sort}
        onchange={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          if (Number.isNaN(v)) return;
          setSort(v);
        }} /></label>

    {#if parentName !== undefined}
      <button type="button" onclick={() => ctx.openDocument({ docId: doc?.parent_id ?? "" })}>
        {t("sheetNote.upTo")} {parentName}
      </button>
    {/if}

    {#if children.length > 0}
      <h3>{t("sheetNote.children")}</h3>
      <ul class="children">
        {#each children as child (child.id)}
          <li><button type="button" data-testid="note-child" data-note-id={child.id} onclick={() => ctx.openDocument({ docId: child.id })}>{child.name}</button></li>
        {/each}
      </ul>
    {/if}

    {#if ctx.canCreate(NOTE_DOC_TYPE)}
      <button type="button" data-testid="note-new-child" onclick={newChild}>{t("sheetNote.newChild")}</button>
    {/if}
  {:else}
    <p class="missing">{t("sheets.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .close:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  label { display: flex; flex-direction: column; gap: 2px; }
  .body { border: 1px solid var(--border); border-radius: var(--radius-1); padding: var(--space-1); }
  .unrenderable, .missing { opacity: 0.7; font-style: italic; }
  .changed-remotely { color: var(--warning, #b45309); }
  .editor-actions { display: flex; gap: var(--space-1); }
  .children { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .children button { min-height: 44px; text-align: left; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
</style>
