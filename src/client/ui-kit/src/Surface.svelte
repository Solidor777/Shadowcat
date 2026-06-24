<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "./appContext";

  let { contract }: { contract: string } = $props();

  const { contributions } = getAppContext();

  // Bridge the framework-neutral registry's subscribe/snapshot to Svelte's
  // reactivity: reading `subscribe()` inside the $derived registers a dependency
  // that re-runs whenever the registry emits.
  const subscribe = createSubscriber((update) => {
    const off = contributions.subscribe(update);
    return () => off();
  });

  const items = $derived.by(() => {
    subscribe();
    return contributions.contributionsFor(contract);
  });
</script>

{#each items as item (item.id)}
  {@const Comp = item.component as Component<Record<string, unknown>>}
  <Comp {...(item.props ?? {})} />
{/each}
