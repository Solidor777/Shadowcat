<script lang="ts">
  /** Test fixture: throws during a reactive re-render triggered by a click —
   * not directly inside the raw DOM event handler — so the throw runs inside
   * a Svelte-owned effect and reaches the nearest `svelte:boundary`. */
  let boom = $state(false);
</script>

<button type="button" data-testid="boom-btn" onclick={() => (boom = true)}>boom</button>
{#if boom}
  {(() => {
    throw new Error("boom");
  })()}
{/if}
