<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveSessionPause } from "@shadowcat/core";

  const ctx = getAppContext();
  // Reactive bridge: `ctx.documents` is a plain-callback store, not a rune, so the pause state
  // would never re-read after the world-settings document changes without it.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const pause = $derived.by(() => {
    subscribe();
    return resolveSessionPause(ctx.documents);
  });
</script>

{#if pause.paused}
  <!-- The GM's message is rendered as a text node (never `{@html}`), so markup in it stays inert. -->
  <p class="pause-banner" role="status" data-testid="session-pause-banner">
    {#if pause.message}
      {ctx.t("topbar.pause.bannerWithMessage", { message: pause.message })}
    {:else}
      {ctx.t("topbar.pause.banner")}
    {/if}
  </p>
{/if}

<style lang="scss">
  .pause-banner {
    margin: 0;
    padding: var(--space-1) var(--space-2);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--warning, var(--border));
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    font-weight: 600;
  }
</style>
