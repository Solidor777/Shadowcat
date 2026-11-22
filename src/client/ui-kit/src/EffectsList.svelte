<script lang="ts">
  // Read-only list of the effects on one actor or token host, each with its origin. Whoever
  // can read an effect document reads its origin: the store already holds only what the
  // server sent this recipient, so nothing is filtered here.
  import { createSubscriber } from "svelte/reactivity";
  import type { WireDocument } from "@shadowcat/core";
  import { getAppContext } from "./appContext";
  import { listEffects, type EffectEntry } from "./effectsList";

  /** EffectsList props. */
  interface Props {
    /** The actor or token document whose effects to list; `undefined` renders nothing. */
    host?: WireDocument | undefined;
    /** A combatant id; effects on other hosts anchored to it are listed too. */
    anchor?: string | undefined;
    /** Renders the compact inline form used inside a tracker row. */
    compact?: boolean;
    /** A heading shown above a non-empty list; omitted renders none. */
    heading?: string | undefined;
  }
  const { host, anchor, compact = false, heading }: Props = $props();

  const ctx = getAppContext();
  // `ctx.documents` is a plain-callback store, not a rune: without this bridge an effect
  // added, expired or relabelled during play never updates the list.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const entries = $derived.by((): EffectEntry[] => {
    subscribe();
    if (!host) return [];
    const live = ctx.documents.get(host.id) ?? host;
    return listEffects(live, { get: (id) => ctx.documents.get(id), query: (t) => ctx.documents.query(t) }, ctx.t("effects.unnamed"), anchor);
  });
</script>

{#if entries.length > 0}
  {#if heading}<h3>{heading}</h3>{/if}
  <ul class="effects" class:compact aria-label={ctx.t("effects.title")} data-testid="effects-list">
    {#each entries as e (e.id)}
      <li class="effect" class:inactive={!e.active} data-testid={"effects-list:" + e.id}>
        <span class="effect-name">{e.name}</span>
        {#if e.origin}
          <span class="effect-origin">{ctx.t("effects.origin", { origin: e.origin })}</span>
        {/if}
        {#if !e.active}
          <span class="effect-state">{ctx.t("effects.inactive")}</span>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style lang="scss">
  .effects { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .effects.compact { flex-direction: row; flex-wrap: wrap; gap: var(--space-1); }
  .effect { display: flex; flex-wrap: wrap; gap: var(--space-1); align-items: baseline; }
  .effect.inactive .effect-name { opacity: 0.5; text-decoration: line-through; }
  .effect-origin, .effect-state { opacity: 0.7; font-size: 0.85em; }
</style>
