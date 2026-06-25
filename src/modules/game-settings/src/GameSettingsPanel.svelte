<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc } from "@shadowcat/core";

  const ctx = getAppContext();

  // Reactive subscription: mirrors the established registry-seed pattern (FactionsPanel/ConditionsPanel).
  // Calling subscribe() inside the effect registers a reactive dependency on the document store so
  // the effect re-evaluates after the resync stream populates it post-mount.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  let seeded = false;

  // Idempotent GM seed: create world-settings, light-gradation, and vision-modes once, only when
  // absent. The per-doc-type length === 0 guard prevents duplicate creation once the store is
  // populated. The reactive subscription ensures re-evaluation after resync lands.
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    const ops = [];
    if (ctx.documents.query("world-settings").length === 0) ops.push({ op: "create" as const, doc: buildWorldSettingsDoc(ctx.world) });
    if (ctx.documents.query("light-gradation").length === 0) ops.push({ op: "create" as const, doc: buildLightGradationDoc(ctx.world) });
    if (ctx.documents.query("vision-modes").length === 0) ops.push({ op: "create" as const, doc: buildVisionModesDoc(ctx.world) });
    seeded = true;
    if (ops.length > 0) ctx.dispatchIntent(ops);
  });
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>
</section>
