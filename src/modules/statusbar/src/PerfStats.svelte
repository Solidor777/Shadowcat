<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  const { performance } = getAppContext();
</script>

{#if performance.showStats}
  <span class="perf-stats" data-testid="perf-stats">
    {performance.stats.fps} fps · {performance.stats.frameMs.toFixed(1)} ms
  </span>
{/if}

<style lang="scss">
  .perf-stats {
    font-size: var(--font-size-caption);
    color: var(--text-muted);
    white-space: nowrap;
  }
</style>
