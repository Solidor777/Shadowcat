<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, SegmentList, PermissionsEditor } from "@shadowcat/ui-kit";
  import { getPointer, firstChannel, parseNoteBody, parseNotePageKind, buildNoteDoc, appendNoteSortKey, NOTE_DOC_TYPE, type WireDocument } from "@shadowcat/core";
  import { collectHeadings, findAnchor, type TocEntry } from "./note-toc";

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
  /** The headings read from the rendered body, in order; drives the table of contents. */
  let headings = $state.raw<TocEntry[]>([]);
  /** The body fieldset; heading collection and anchor scrolling are scoped to it. */
  let bodyEl = $state<HTMLElement | null>(null);

  // The rendered headings are sanitized markup with no ids. The table of contents and the ids
  // both come from the rendered heading elements, so they cannot disagree. Re-runs whenever the
  // body changes, after the new markup is in the DOM.
  $effect(() => {
    void body;
    headings = bodyEl ? collectHeadings(bodyEl) : [];
  });

  // The body's fragment links are plain anchors inside `{@html}` markup, so the click listener
  // is attached to the element rather than declared in the template.
  $effect(() => {
    const el = bodyEl;
    if (!el) return;
    el.addEventListener("click", onBodyClick);
    return () => el.removeEventListener("click", onBodyClick);
  });

  /** Scrolls the heading with id `slug` into view inside the note body.
   * @param slug The heading anchor id.
   * @example
   * ```
   * // private function; not part of the public API — wired to TOC and in-body fragment links
   * scrollToHeading("intro");
   * ```
   */
  function scrollToHeading(slug: string): void {
    if (bodyEl) findAnchor(bodyEl, slug)?.scrollIntoView({ block: "start" });
  }

  /** Handles a click on a same-page fragment link in the rendered body: the hash router owns
   * `location.hash`, so the link scrolls inside the note instead of navigating.
   * @param event The click event bubbling from the body.
   * @example
   * ```
   * // private function; not part of the public API — wired to the body fieldset's click
   * declare const event: MouseEvent;
   * onBodyClick(event);
   * ```
   */
  function onBodyClick(event: MouseEvent): void {
    const link = (event.target as Element).closest("a");
    const href = link?.getAttribute("href");
    if (!href || !href.startsWith("#")) return;
    event.preventDefault();
    scrollToHeading(href.slice(1));
  }
  const kindPath = $derived(`${enginePrefix}/kind`);
  const pageKind = $derived.by(() => (doc ? parseNotePageKind(doc) : ({ kind: "text" } as const)));
  /** The media asset id of an image/pdf/video page, or `null` for a text page. */
  const mediaAssetId = $derived(pageKind.kind === "text" ? null : pageKind.asset_id);
  /** The serve URL for the media asset; the resolver's placeholder when the asset was deleted. */
  const mediaUrl = $derived(mediaAssetId === null ? null : ctx.assets.url(mediaAssetId));
  /** Set by the media element's error event; reset whenever the asset id changes. */
  let mediaErrored = $state(false);
  let erroredFor = $state<string | null>(null);
  /** True when the media cannot be shown: the resolver gave its placeholder, or the element failed to load. */
  const mediaFailed = $derived(
    mediaUrl === ctx.assets.placeholder() || (mediaErrored && erroredFor === mediaAssetId),
  );

  const canEditVisibility = $derived(!!doc && ctx.canEdit(doc, "/permissions/default"));
  const canEditSource = $derived(!!doc && ctx.canEdit(doc, sourcePath));
  const canEditKind = $derived(!!doc && ctx.canEdit(doc, kindPath));

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

  /** Records that the media element for the current asset failed to load.
   * @example
   * ```
   * // private function; not part of the public API — wired to the media element's error event
   * markMediaErrored();
   * ```
   */
  function markMediaErrored(): void {
    mediaErrored = true;
    erroredFor = mediaAssetId;
  }

  /** Opens the shared asset picker and swaps the page's media asset, keeping its kind. Cancel
   * writes nothing. The picker's `kind` filter is `image` for an image page and `other` for a
   * PDF or video page.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "Change media" button
   * void changeMedia();
   * ```
   */
  async function changeMedia(): Promise<void> {
    if (!doc || pageKind.kind === "text") return;
    const picked = await ctx.pickAsset({ kind: pageKind.kind === "image" ? "image" : "other" });
    if (!picked) return;
    setField(ctx, docId, kindPath, pageKind, { kind: pageKind.kind, asset_id: picked });
  }

  /** Creates a new child note under this one and opens it.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "New child note" button
   * newChild();
   * ```
   */
  function newChild(): void {
    const sort = appendNoteSortKey(ctx.documents.query(NOTE_DOC_TYPE), docId);
    const child = buildNoteDoc(ctx.world, t("sheetNote.untitled"), "", { parentId: docId, owner: ctx.selfId, sort });
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

    <PermissionsEditor {docId} />

    {#if mediaAssetId !== null && mediaUrl !== null}
      <div class="media" data-testid="note-media" data-media-kind={pageKind.kind}>
        {#if mediaFailed}
          <p class="media-missing" data-testid="note-media-missing">{t("sheetNote.mediaMissing")}</p>
        {:else if pageKind.kind === "image"}
          <img src={mediaUrl} alt={name ?? ""} onerror={markMediaErrored} />
        {:else if pageKind.kind === "pdf"}
          <embed type="application/pdf" src={mediaUrl} title={name ?? t("sheetNote.title")} />
        {:else}
          <video src={mediaUrl} controls preload="metadata" onerror={markMediaErrored}>
            <track kind="captions" />
          </video>
        {/if}
        {#if canEditKind}
          <button type="button" data-testid="note-change-media" onclick={() => void changeMedia()}>{t("sheetNote.changeMedia")}</button>
        {/if}
      </div>
    {/if}

    {#if headings.length > 0}
      <nav class="toc" data-testid="note-toc" aria-label={t("sheetNote.toc")}>
        <ul>
          {#each headings as h (h.slug)}
            <li style:margin-inline-start="{(h.level - 1) * 12}px">
              <a href="#{h.slug}" data-testid="note-toc-link" onclick={(e) => { e.preventDefault(); scrollToHeading(h.slug); }}>{h.text}</a>
            </li>
          {/each}
        </ul>
      </nav>
    {/if}

    <fieldset class="body" disabled={(channel ?? "") === ""} data-testid="note-body" bind:this={bodyEl}>
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
  .toc ul { list-style: none; margin: 0; padding: 0; }
  .toc a { display: inline-block; min-height: 44px; line-height: 44px; }
  .media { display: flex; flex-direction: column; gap: var(--space-1); }
  .media img, .media video { max-width: 100%; height: auto; }
  .media embed { width: 100%; min-height: 60vh; }
  .media button { min-height: 44px; }
  .media-missing { opacity: 0.7; font-style: italic; }
  .unrenderable, .missing { opacity: 0.7; font-style: italic; }
  .changed-remotely { color: var(--warning, #b45309); }
  .editor-actions { display: flex; gap: var(--space-1); }
  .children { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .children button { min-height: 44px; text-align: left; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
</style>
