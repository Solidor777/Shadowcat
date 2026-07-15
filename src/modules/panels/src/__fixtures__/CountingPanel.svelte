<script lang="ts">
  import { onMount } from "svelte";

  /** Test fixture: reports exactly one call per component mount, independent
   * of prop changes or reactive re-runs, to prove keep-mounted-by-construction. */
  let { onMountFn }: { onMountFn: () => void } = $props();

  onMount(() => {
    onMountFn();
  });
</script>

<div data-testid="counting-panel">counting</div>
