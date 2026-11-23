<script lang="ts">
  // Controlled editor for an ordered list of attribute bars. Each bar is a label, a `current` and
  // a `max` (a number or a formula over the token's actor), a colour and a display mode. The host
  // owns the list: every committed edit calls `onCommit` with the whole next list, so one write
  // carries every bar's change. A formula is checked with `parseFormula` for an inline error
  // only; the server stays the authority on what a formula means.
  import { getAppContext } from "@shadowcat/ui-kit";
  import { parseFormula } from "@shadowcat/formula";
  import { MAX_BARS, MAX_BAR_LABEL_CHARS, type BarAttribute, type BarDisplayMode, type Formula } from "@shadowcat/core";
  import BarModeSelect from "./BarModeSelect.svelte";

  const ctx = getAppContext();
  const t = ctx.t;

  let { bars, onCommit }: {
    /** The authored bar list to edit, in order. */
    bars: BarAttribute[];
    /** Called with the whole next list after each committed edit, add or remove. */
    onCommit: (next: BarAttribute[]) => void;
  } = $props();

  /** Inline error text keyed `<index>:<field>`, set by a rejected edit and cleared by the next
   * accepted one. */
  let errors = $state<Record<string, string | null>>({});

  /** Coerces a text input into a `Formula`: a finite numeric literal becomes a number, else a
   * valid formula string is kept trimmed, else `null`.
   * @param text The raw input value.
   * @returns The coerced `Formula`, or `null` when `text` is neither a number nor a valid formula.
   * @example
   * ```
   * coerceFormula("2"); // 2
   * ```
   */
  function coerceFormula(text: string): Formula | null {
    const trimmed = text.trim();
    const n = Number(trimmed);
    if (trimmed !== "" && Number.isFinite(n)) return n;
    const parsed = parseFormula(trimmed);
    if ("error" in parsed) return null;
    return trimmed;
  }

  /** Replaces bar `index` with `patch` merged over it and commits the list.
   * @param index The bar's position.
   * @param patch The fields to change.
   * @example
   * ```
   * // private helper; wired to each field's change handler
   * patchBar(0, { color: "#00ff00" });
   * ```
   */
  function patchBar(index: number, patch: Partial<BarAttribute>): void {
    onCommit(bars.map((b, i) => (i === index ? { ...b, ...patch } : b)));
  }

  /** Commits a label edit, or surfaces the inline error when the label is empty.
   * @param index The bar's position.
   * @param text The raw input value.
   * @example
   * ```
   * // private helper; wired to the label input's change handler
   * commitLabel(0, "HP");
   * ```
   */
  function commitLabel(index: number, text: string): void {
    const trimmed = text.trim();
    errors[`${index}:label`] = trimmed === "" ? t("actors.barLabelRequired") : null;
    if (trimmed !== "") patchBar(index, { label: trimmed });
  }

  /** Commits a `current`/`max` edit, or surfaces the inline error when the text is neither a
   * number nor a valid formula.
   * @param index The bar's position.
   * @param field Which formula field the text edits.
   * @param text The raw input value.
   * @example
   * ```
   * // private helper; wired to the formula inputs' change handlers
   * commitFormula(0, "current", "hp.value");
   * ```
   */
  function commitFormula(index: number, field: "current" | "max", text: string): void {
    const coerced = coerceFormula(text);
    errors[`${index}:${field}`] = coerced === null ? t("actors.barInvalidFormula", { detail: text.trim() }) : null;
    if (coerced !== null) patchBar(index, { [field]: coerced });
  }

  /** Appends a default bar.
   * @example
   * ```
   * // private helper; wired to the add button
   * addBar();
   * ```
   */
  function addBar(): void {
    onCommit([...bars, { label: t("actors.barDefaultLabel"), current: 0, max: 1, color: "#cc2222", mode: "hover" }]);
  }

  /** Removes bar `index`.
   * @param index The bar's position.
   * @example
   * ```
   * // private helper; wired to each remove button
   * removeBar(0);
   * ```
   */
  function removeBar(index: number): void {
    onCommit(bars.filter((_, i) => i !== index));
  }
</script>

<div class="bars-editor" data-testid="bars-editor">
  {#each bars as bar, i (i)}
    <div class="bar-row" data-testid="bar-row">
      <input
        type="text" maxlength={MAX_BAR_LABEL_CHARS} aria-label={t("actors.barLabel")} data-testid="bar-label"
        value={bar.label} onchange={(e) => commitLabel(i, e.currentTarget.value)}
      />
      <input
        type="text" aria-label={t("actors.barCurrent")} data-testid="bar-current"
        value={String(bar.current)} onchange={(e) => commitFormula(i, "current", e.currentTarget.value)}
      />
      <input
        type="text" aria-label={t("actors.barMax")} data-testid="bar-max"
        value={String(bar.max)} onchange={(e) => commitFormula(i, "max", e.currentTarget.value)}
      />
      <input
        type="color" aria-label={t("actors.barColor")} data-testid="bar-color"
        value={bar.color} onchange={(e) => patchBar(i, { color: e.currentTarget.value })}
      />
      <BarModeSelect
        value={bar.mode} label={t("actors.barMode")} testId="bar-mode"
        onChange={(mode: BarDisplayMode | null) => mode !== null && patchBar(i, { mode })}
      />
      <button type="button" data-testid="bar-remove" aria-label={t("actors.barRemove", { label: bar.label })} onclick={() => removeBar(i)}>×</button>
      {#each ["label", "current", "max"] as field (field)}
        {#if errors[`${i}:${field}`]}<p class="error" role="alert">{errors[`${i}:${field}`]}</p>{/if}
      {/each}
    </div>
  {/each}
  <button type="button" data-testid="bar-add" disabled={bars.length >= MAX_BARS} onclick={addBar}>{t("actors.barAdd")}</button>
  {#if bars.length >= MAX_BARS}<p class="hint">{t("actors.barMaxReached", { max: MAX_BARS })}</p>{/if}
</div>

<style lang="scss">
  .bars-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .bar-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    align-items: center;
  }
  .bar-row input[type="text"] {
    min-height: 44px;
    flex: 1 1 8em;
  }
  .bar-row input[type="color"],
  button {
    min-height: 44px;
    min-width: 44px;
  }
  .error {
    flex-basis: 100%;
    margin: 0;
    color: var(--danger, #f85149);
    font-size: 0.85em;
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
</style>
