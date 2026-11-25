<script lang="ts">
  import { onMount } from "svelte";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TourController } from "./controller.svelte";
  import { appEnvironment } from "./environment";
  import TourHint from "./TourHint.svelte";
  import TourOffer from "./TourOffer.svelte";
  import TourOverlay from "./TourOverlay.svelte";

  let {
    controller,
  }: {
    /** The session's tour controller. */
    controller: TourController;
  } = $props();

  const ctx = getAppContext();

  // The offer is decided once the surfaces are mounted (so step targets exist) and again whenever
  // a module contributes or removes a tour; the controller makes at most one offer per session.
  onMount(() => {
    const detach = controller.attach(appEnvironment(ctx, document));
    void controller.evaluate();
    const unsubscribe = ctx.contributions.subscribe(() => void controller.evaluate());
    return () => {
      unsubscribe();
      detach();
    };
  });
</script>

{#if controller.offer !== null}
  <TourOffer {controller} />
{/if}
{#if controller.howTo}
  <TourHint {controller} />
{/if}
{#if controller.active !== null}
  <TourOverlay {controller} />
{/if}
