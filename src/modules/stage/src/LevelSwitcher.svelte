<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { SceneLevel } from "@shadowcat/core";

  const { t } = getAppContext();

  let {
    levels,
    active,
    onSelect,
  }: {
    /** The viewed scene's declared floors, in authored order. */
    levels: SceneLevel[];
    /** The currently-viewed level id, or `null`. */
    active: string | null;
    /** Called with a level's id when its button is clicked. */
    onSelect: (id: string) => void;
  } = $props();
</script>

{#if levels.length > 0}
  <div class="level-switcher" data-testid="level-switcher" role="group" aria-label={t("levels.switcher")}>
    {#each levels as level (level.id)}
      <button
        type="button"
        class="level-button"
        data-testid={`level-${level.id}`}
        aria-pressed={level.id === active}
        onclick={() => onSelect(level.id)}
      >
        {level.name}
      </button>
    {/each}
  </div>
{/if}

<style lang="scss">
  .level-switcher {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    display: flex;
    gap: var(--space-1);
  }
  .level-button {
    min-height: var(--input-height-coarse);
    min-width: var(--input-height-coarse);
    padding: var(--space-1) var(--space-2);
    font-size: 0.8125rem;
    color: var(--text-primary);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    cursor: pointer;

    &[aria-pressed="true"] {
      background: var(--accent);
      color: var(--on-accent);
    }
  }
</style>
