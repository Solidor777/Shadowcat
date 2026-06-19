<script lang="ts">
  import { ContributionRegistry } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  // The registry is a fixed instance per render; capturing it once is intended.
  // svelte-ignore state_referenced_locally
  setAppContext({ contributions: registry });
</script>

<Surface {contract} />
