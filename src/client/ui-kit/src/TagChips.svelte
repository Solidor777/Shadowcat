<script lang="ts">
  // Shared free-form tag chip editor: one removable chip per tag plus a text input that commits a
  // deduplicated, trimmed tag on Enter. Value-only and string-prop driven (no i18n keys of its
  // own), so the asset browser's filter bar and the random-variant pool editor reuse one control
  // with their own wording and test ids.

  let {
    tags,
    onChange,
    placeholder,
    removeLabel,
    testIdPrefix,
  }: {
    /** The tags being edited. */
    tags: string[];
    /** Called with the whole next tag list on every add or remove. */
    onChange: (next: string[]) => void;
    /** Placeholder and accessible name of the add input. */
    placeholder: string;
    /** Builds a remove button's accessible name from the tag it removes. */
    removeLabel: (tag: string) => string;
    /** Test id prefix: the input is `<prefix>-input`, each remove button `<prefix>-remove-<tag>`. */
    testIdPrefix: string;
  } = $props();

  /** The pending tag in the add input. */
  let draft = $state("");

  /** Commits the draft as a chip: trimmed, non-empty and not already present.
   * @example
   * ```
   * // private function; wired to the add input's Enter handler below
   * commitDraft();
   * ```
   */
  function commitDraft(): void {
    const tag = draft.trim();
    draft = "";
    if (!tag || tags.includes(tag)) return;
    onChange([...tags, tag]);
  }
</script>

{#each tags as tag (tag)}
  <span class="chip">
    {tag}
    <button
      type="button"
      data-testid={testIdPrefix + "-remove-" + tag}
      aria-label={removeLabel(tag)}
      onclick={() => onChange(tags.filter((x) => x !== tag))}
    >×</button>
  </span>
{/each}
<input
  data-testid={testIdPrefix + "-input"}
  type="text"
  {placeholder}
  aria-label={placeholder}
  bind:value={draft}
  onkeydown={(e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitDraft();
    }
  }}
/>

<style lang="scss">
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.125rem;
    padding: 0.125rem 0.375rem;
    border: 1px solid var(--border);
    border-radius: 1rem;
    font-size: 0.8rem;
    button {
      min-height: 0;
      border: none;
      background: none;
      cursor: pointer;
      color: var(--text-muted);
    }
  }
  input {
    min-height: 2rem;
  }
</style>
