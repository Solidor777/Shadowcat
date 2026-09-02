<script lang="ts">
  // Shared movement-tag chip editor: a toggle chip per engine-reserved tag plus one removable
  // chip per free-form tag, plus an add row. Deliberately value-only — the OWNING surface
  // (actor row, actor sheet, faction row, token-override control) decides what the list means
  // around it and owns the dispatch, so the raw-`old` OCC pre-image read stays at the call
  // site that knows the document path.
  import { getAppContext } from "./appContext";

  const ctx = getAppContext();
  const t = ctx.t;

  /** The tags the engine itself interprets — each means the mover ignores difficult-terrain
   * COST and nothing else (`ActorEngine.movement` states the semantics). Offered as
   * first-class toggle chips; every other tag is inert system vocabulary added free-form. */
  const RESERVED_TAGS = ["flying", "incorporeal"] as const;

  let { value, disabled = false, onCommit }: {
    /** The tags being edited (never `null` — the parent normalizes its stored value before
     * rendering; the committed list is passed through verbatim, INCLUDING when empty). */
    value: string[];
    /** Read-only mode (a sheet the viewer may not edit). */
    disabled?: boolean;
    /** Commit callback: receives the WHOLE updated tag list (whole-payload replacement). */
    onCommit: (next: string[]) => void;
  } = $props();

  /** The stored tags deduplicated for display: the list is a SET semantically
   * (`resolveTokenActor` dedups on resolution), so a duplicate stored by another writer must
   * never render as two chips — and toggling one off removes every occurrence. */
  const tags = $derived([...new Set(value)]);
  /** The free-form tags (everything not engine-reserved) rendered as removable chips; a
   * reserved tag is removed by untoggling its chip instead. */
  const customTags = $derived(tags.filter((tag) => !(RESERVED_TAGS as readonly string[]).includes(tag)));

  /** The pending free-form tag in the add row's input. */
  let draft = $state("");

  /** Toggle one engine-reserved tag: on appends it, off removes every occurrence.
   * @param tag The reserved tag to toggle.
   * @example
   * ```
   * // private helper; wired to each reserved chip's onclick
   * toggleReserved("flying");
   * ```
   */
  function toggleReserved(tag: string): void {
    onCommit(tags.includes(tag) ? tags.filter((x) => x !== tag) : [...tags, tag]);
  }

  /** Remove one tag (every occurrence — see `tags`).
   * @param tag The tag to remove.
   * @example
   * ```
   * // private helper; wired to each custom chip's remove button
   * removeTag("burrowing");
   * ```
   */
  function removeTag(tag: string): void {
    onCommit(tags.filter((x) => x !== tag));
  }

  /** Commit the add row's draft as a new tag: trimmed; an empty draft or an exact duplicate
   * (reserved or custom) is a no-op — fail-closed, never widening the set on garbled input.
   * @example
   * ```
   * // private helper; wired to the add button's onclick + the input's Enter keydown
   * addDraft();
   * ```
   */
  function addDraft(): void {
    const tag = draft.trim();
    if (tag === "" || tags.includes(tag)) return;
    onCommit([...tags, tag]);
    draft = "";
  }
</script>

<div class="movement-tags">
  <div class="chips">
    {#each RESERVED_TAGS as tag (tag)}
      <button
        type="button"
        class="chip toggle"
        class:active={tags.includes(tag)}
        aria-pressed={tags.includes(tag)}
        data-testid="movement-toggle-{tag}"
        {disabled}
        onclick={() => toggleReserved(tag)}
      >{tag}</button>
    {/each}
    {#each customTags as tag (tag)}
      <span class="chip">
        {tag}
        <button
          type="button"
          aria-label={t("actors.movementRemove", { tag })}
          data-testid="movement-remove-{tag}"
          {disabled}
          onclick={() => removeTag(tag)}
        >×</button>
      </span>
    {/each}
  </div>
  <div class="add-row">
    <input
      aria-label={t("actors.movementTag")}
      placeholder={t("actors.movementTag")}
      data-testid="movement-input"
      bind:value={draft}
      {disabled}
      onkeydown={(e) => {
        // Enter inside a hosting form (the actor create form) must add the tag, not submit.
        if (e.key === "Enter") { e.preventDefault(); addDraft(); }
      }}
    />
    <button type="button" data-testid="movement-add" disabled={disabled || draft.trim() === ""} onclick={addDraft}>
      {t("actors.movementAdd")}
    </button>
  </div>
</div>

<style lang="scss">
  .movement-tags {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-height: 32px;
    padding: 0 var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
  }
  button.chip {
    cursor: pointer;
  }
  .chip.active {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
  .chip button {
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
    min-width: 24px;
    min-height: 24px;
  }
  .add-row {
    display: flex;
    gap: var(--space-1);
  }
  .movement-tags input,
  .movement-tags .add-row button {
    min-height: 32px;

    @media (pointer: coarse) {
      min-height: 44px;
    }
  }
</style>
