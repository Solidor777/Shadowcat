<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField, setFields } from "@shadowcat/ui-kit";
  import { getPointer, firstChannel, type WireDocument, type TableEngine, type TableRow, type DrawRule } from "@shadowcat/core";
  import { addRow, removeRow, moveRow, setRow, normalizeRowsForDraw } from "./rowOps";
  import RowEditor from "./RowEditor.svelte";

  // Rollable-table sheet: name/description/draw-rule fields, a rows editor (whole-array
  // replace on every mutation — `set_pointer` cannot grow arrays), and a Draw affordance that
  // posts a chat draw. Reads the OPTIMISTIC store; every write's `old` is the RAW current
  // stored value. `readOnly` gates every write control; Draw is gated separately by
  // `channel === null` (a missing channel-registry, independent of write permission).
  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The table document this sheet edits (a table is never embedded, never parented). */
    docId: string;
    /** The write root for the opaque `system` tree; `basePrefix`/`enginePrefix`/`namePrefix`
     * below are derived from it, same pattern as every other sheet. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to the header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const enginePrefix = $derived(`${basePrefix}/engine`);
  const namePrefix = $derived(`${basePrefix}/name`);
  const rowsPath = $derived(`${enginePrefix}/rows`);
  const drawPath = $derived(`${enginePrefix}/draw`);
  const descriptionPath = $derived(`${enginePrefix}/description`);

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const name = $derived.by((): string | null => (doc ? (getPointer(doc, namePrefix) as string | null | undefined) ?? null : null));
  const engine = $derived.by((): TableEngine | undefined => (doc ? (getPointer(doc, enginePrefix) as TableEngine | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, enginePrefix));
  const channel = $derived.by((): string | null => {
    subscribe();
    return firstChannel(ctx.documents);
  });

  let drawCount = $state(1);

  /** Update the envelope `name` field.
   * @param value The new name.
   * @example
   * ```
   * // private function; not part of the public API — wired to the name input
   * setName("Random Encounters");
   * ```
   */
  function setName(value: string): void {
    if (!doc) return;
    setField(ctx, docId, namePrefix, name, value);
  }

  /** Update `engine/description`.
   * @param value The new description.
   * @example
   * ```
   * // private function; not part of the public API — wired to the description textarea
   * setDescription("A table of minor magic items.");
   * ```
   */
  function setDescription(value: string): void {
    if (!doc || !engine) return;
    setField(ctx, docId, descriptionPath, engine.description, value);
  }

  /** Update `engine/draw` (the whole rule object) together with `engine/rows`, reshaped for
   * the new rule via `normalizeRowsForDraw` — `TableEngine::validate` requires `range: None`
   * under `Weighted` and `range: Some` under `Formula` for EVERY row, checked against the
   * whole post-image, so switching rules while an existing row's `range` disagrees would be
   * rejected by the server. Dispatched as ONE atomic Update (`setFields`) carrying both
   * `FieldChange`s so the post-image is never a rule/rows mismatch.
   * @param value The replacement draw rule.
   * @example
   * ```
   * // private function; not part of the public API — wired to the draw-rule select
   * setDraw({ kind: "weighted" });
   * ```
   */
  function setDraw(value: DrawRule): void {
    if (!doc || !engine) return;
    setFields(ctx, docId, [
      { path: drawPath, old: engine.draw, value },
      { path: rowsPath, old: engine.rows, value: normalizeRowsForDraw(engine.rows, value) },
    ]);
  }

  /** Update `engine/rows` (the whole array).
   * @param value The replacement rows array.
   * @example
   * ```
   * // private function; not part of the public API — wired to every row mutation below
   * declare const rows: TableRow[];
   * setRows(rows);
   * ```
   */
  function setRows(value: TableRow[]): void {
    if (!doc || !engine) return;
    setField(ctx, docId, rowsPath, engine.rows, value);
  }

  /** Draws `drawCount` rows into chat over `channel`. Surfaces the server's
   * player-presentable refusal reason (over-cap count, empty table, cycle, missing asset, rate
   * limit) via `ctx.notify` — the count input carries no client-side maximum, matching the
   * server's own definition of the cap.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Draw button
   * draw();
   * ```
   */
  function draw(): void {
    if (channel === null) return;
    ctx.chat.drawTable({ tableId: docId, channel, count: drawCount }).catch((e) => {
      ctx.notify(String(e instanceof Error ? e.message : e));
    });
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{name ?? t("sheetTable.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc && engine}
    <label>{t("sheetTable.title")}
      <input data-testid="table-name" aria-label={t("sheetTable.title")} value={name ?? ""} disabled={readOnly}
        onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>

    <div class="draw-controls">
      <label>{t("sheetTable.drawCount")}
        <input data-testid="table-draw-count" type="number" min="1" aria-label={t("sheetTable.drawCount")}
          value={drawCount} onchange={(e) => {
            const v = Number((e.currentTarget as HTMLInputElement).value);
            if (!Number.isNaN(v) && v > 0) drawCount = v;
          }} /></label>
      <button type="button" data-testid="table-draw" disabled={channel === null} onclick={draw}>{t("sheetTable.draw")}</button>
      {#if channel === null}<span class="no-channel">{t("sheetTable.noChannel")}</span>{/if}
    </div>

    <label>{t("sheetTable.description")}
      <textarea data-testid="table-description" aria-label={t("sheetTable.description")} disabled={readOnly}
        value={engine.description} onchange={(e) => setDescription((e.currentTarget as HTMLTextAreaElement).value)}></textarea></label>

    <label>{t("sheetTable.drawRule")}
      <select data-testid="table-draw-rule" aria-label={t("sheetTable.drawRule")} value={engine.draw.kind} disabled={readOnly}
        onchange={(e) => {
          const kind = (e.currentTarget as HTMLSelectElement).value;
          setDraw(kind === "formula" ? { kind: "formula", notation: "1d20" } : { kind: "weighted" });
        }}>
        <option value="weighted">{t("sheetTable.weighted")}</option>
        <option value="formula">{t("sheetTable.formula")}</option>
      </select></label>

    {#if engine.draw.kind === "formula"}
      <label>{t("sheetTable.notation")}
        <input data-testid="table-notation" aria-label={t("sheetTable.notation")} disabled={readOnly}
          value={engine.draw.notation} onchange={(e) => setDraw({ kind: "formula", notation: (e.currentTarget as HTMLInputElement).value })} /></label>
    {/if}

    <h3>{t("sheetTable.rows")}</h3>
    {#each engine.rows as row, i (i)}
      <RowEditor
        {row}
        draw={engine.draw}
        disabled={readOnly}
        onChange={(next) => setRows(setRow(engine!.rows, i, next))}
        onRemove={() => setRows(removeRow(engine!.rows, i))}
        onMoveUp={i > 0 ? () => setRows(moveRow(engine!.rows, i, -1)) : undefined}
        onMoveDown={i < engine.rows.length - 1 ? () => setRows(moveRow(engine!.rows, i, 1)) : undefined}
      />
    {/each}
    <button type="button" data-testid="table-add-row" disabled={readOnly} onclick={() => setRows(addRow(engine!.rows, engine!.draw))}>{t("sheetTable.addRow")}</button>
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
  .draw-controls { display: flex; align-items: center; gap: var(--space-1); }
  .no-channel { opacity: 0.7; font-style: italic; }
  .missing { opacity: 0.7; font-style: italic; }
</style>
