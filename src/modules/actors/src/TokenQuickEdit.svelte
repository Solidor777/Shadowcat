<script lang="ts">
  // Quick-edit section for the single selected token: elevation, condition toggles and a nudge
  // row per attribute bar, in one compact block. Every part reuses an existing control or the
  // ordinary Update path: elevation is `TokenElevationControl`, the palette is
  // `ConditionTogglePalette`, and a bar nudge writes the `system` leaf the bar's `current` formula
  // names. The rows come from the server's `"bars"` channel, the same redacted delivery the canvas
  // draws, so a bar this viewer is not sent never appears here.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, ConditionTogglePalette } from "@shadowcat/ui-kit";
  import { buildUpdate, type WireDocument } from "@shadowcat/core";
  import TokenElevationControl from "./TokenElevationControl.svelte";
  import { resolveQuickBars, type QuickBar } from "./barQuickEdit";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The single selected token's id, or `null` when no single token is selected — the section
     * renders nothing then. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as the sibling per-token controls).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const token = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  const rows = $derived.by((): QuickBar[] => {
    subscribe();
    return token ? resolveQuickBars(token, ctx.documents, ctx.bars) : [];
  });

  /** How much a `−`/`+` press changes a bar by. */
  let step = $state(1);

  /** Whether the viewer may write the leaf `row` targets (advisory; the server re-checks).
   * @param row The bar row.
   * @returns `true` when the row has a target and the viewer may edit it.
   * @example
   * ```
   * // private helper; read by each row's disabled state
   * declare const row: QuickBar;
   * canWrite(row);
   * ```
   */
  function canWrite(row: QuickBar): boolean {
    return row.target !== null && ctx.canEdit(row.target.doc, row.target.path);
  }

  /** The tooltip that explains a disabled row, or `undefined` for an editable one.
   * @param row The bar row.
   * @returns The explanation text, or `undefined`.
   * @example
   * ```
   * // private helper; read by each row's title attribute
   * declare const row: QuickBar;
   * whyDisabled(row);
   * ```
   */
  function whyDisabled(row: QuickBar): string | undefined {
    if (row.target === null) return t("actors.quickBarComputed");
    return canWrite(row) ? undefined : t("actors.quickBarReadOnly");
  }

  /** Writes `value` to the row's leaf with the delivered current value as the pre-image.
   * @param row The bar row.
   * @param value The new value.
   * @example
   * ```
   * // private helper; wired to the nudge buttons and the value input
   * declare const row: QuickBar;
   * setValue(row, 4);
   * ```
   */
  function setValue(row: QuickBar, value: number): void {
    if (row.target === null || !canWrite(row) || !Number.isFinite(value) || value === row.current) return;
    ctx.dispatchIntent([buildUpdate(row.target.doc.id, [{ path: row.target.path, old: row.current, value }])]);
  }

  /** Commits the value input's text, ignoring anything that is not a number.
   * @param row The bar row.
   * @param raw The input's raw string.
   * @example
   * ```
   * // private helper; wired to the value input's onchange
   * declare const row: QuickBar;
   * commitValue(row, "4");
   * ```
   */
  function commitValue(row: QuickBar, raw: string): void {
    if (raw.trim() === "") return;
    setValue(row, Number(raw));
  }
</script>

{#if token}
  <section class="token-quick-edit" data-testid="token-quick-edit">
    <h4>{t("actors.quickEdit")}</h4>
    <TokenElevationControl {tokenId} />
    <ConditionTogglePalette />
    {#if rows.length > 0}
      <label class="step">
        {t("actors.quickBarStep")}
        <input type="number" min="0" step="any" data-testid="quick-bar-step" bind:value={step} />
      </label>
      {#each rows as row, i (i)}
        {@const reason = whyDisabled(row)}
        <div class="bar-row" data-testid="quick-bar-row">
          <span class="label">{row.label}</span>
          <button type="button" data-testid="quick-bar-dec" title={reason} aria-label={t("actors.quickBarDecrease", { label: row.label })} disabled={reason !== undefined} onclick={() => setValue(row, row.current - step)}>−</button>
          <input
            type="number" step="any" data-testid="quick-bar-value" title={reason} aria-label={t("actors.quickBarValue", { label: row.label })}
            value={row.current} disabled={reason !== undefined} onchange={(e) => commitValue(row, e.currentTarget.value)}
          />
          <button type="button" data-testid="quick-bar-inc" title={reason} aria-label={t("actors.quickBarIncrease", { label: row.label })} disabled={reason !== undefined} onclick={() => setValue(row, row.current + step)}>+</button>
          <span class="max">/ {row.max}</span>
        </div>
      {/each}
    {/if}
  </section>
{/if}

<style lang="scss">
  .token-quick-edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  h4 {
    margin: 0;
  }
  .bar-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
  .label {
    flex: 1 1 6em;
  }
  .bar-row input,
  .step input,
  .bar-row button {
    min-height: 44px;
  }
  .bar-row button {
    min-width: 44px;
  }
  .bar-row input {
    width: 6em;
  }
</style>
