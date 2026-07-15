<script lang="ts">
  import { onMount } from "svelte";

  /** Test fixture: reports one mount per instance via `onMountFn`, and throws
   * during a reactive re-render triggered by a click — each fresh instance
   * starts un-boomed, so a genuine remount recovers without throwing again. */
  let { onMountFn }: { onMountFn: () => void } = $props();

  let boom = $state(false);

  onMount(() => {
    onMountFn();
  });
</script>

<button type="button" data-testid="boom-btn" onclick={() => (boom = true)}>boom</button>
{#if boom}
  {(() => {
    throw new Error("boom");
  })()}
{/if}
