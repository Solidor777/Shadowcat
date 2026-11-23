<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import type { Contribution } from "@shadowcat/core";
  import { getAppContext } from "./appContext";

  let { contract }: {
    /** The contract id to render every current contribution for (e.g. `"shadowcat.panel"`). */
    contract: string;
  } = $props();

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

{#snippet rendered(item: Contribution)}
  {@const Comp = item.component as Component<Record<string, unknown>>}
  {#if item.styling === "isolated"}
    <!-- Theme-isolated contribution: the wrapper re-declares every theme
         token at its engine default for this subtree (see
         `themeIsolationCss`), so the module's own styling applies unaffected
         by the active user theme. -->
    <div class="sc-theme-isolate"><Comp {...(item.props ?? {})} /></div>
  {:else}
    <Comp {...(item.props ?? {})} />
  {/if}
{/snippet}

{#each items as item (item.id)}
  {#if item.tourAnchor}
    <!-- Tour anchor: `display: contents` leaves layout unchanged; `findTourAnchor`
         resolves the anchor to its first element child, which has a box. -->
    <div data-tour={item.tourAnchor} style="display: contents">{@render rendered(item)}</div>
  {:else}
    {@render rendered(item)}
  {/if}
{/each}
