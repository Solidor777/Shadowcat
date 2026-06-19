<script lang="ts">
  import { ContributionRegistry } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  setAppContext({ contributions: registry });
</script>

<Surface {contract} />
