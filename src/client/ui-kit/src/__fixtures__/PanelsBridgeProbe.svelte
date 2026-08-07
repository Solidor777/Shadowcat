<script lang="ts">
  import type { PanelsBridge } from "../panelsBridge.svelte";

  /** Reads `bridge.minimized` through a `$derived` — mirrors a real consumer
   * (e.g. `DockChipsContribution`) that may mount and evaluate BEFORE
   * `bind()` runs, since the panel host mounts later than other AppContext
   * readers. */
  let { bridge }: {
    /** The bridge instance under test; possibly pre-bind at mount time. */
    bridge: PanelsBridge;
  } = $props();
  const minimized = $derived(bridge.minimized);
</script>

<div data-testid="minimized">{minimized.join(",")}</div>
