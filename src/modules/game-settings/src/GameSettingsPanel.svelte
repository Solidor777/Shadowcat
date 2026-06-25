<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc } from "@shadowcat/core";

  const ctx = getAppContext();
  let seeded = false;

  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    seeded = true;
    const ops = [];
    if (ctx.documents.query("world-settings").length === 0) ops.push({ op: "create" as const, doc: buildWorldSettingsDoc(ctx.world) });
    if (ctx.documents.query("light-gradation").length === 0) ops.push({ op: "create" as const, doc: buildLightGradationDoc(ctx.world) });
    if (ctx.documents.query("vision-modes").length === 0) ops.push({ op: "create" as const, doc: buildVisionModesDoc(ctx.world) });
    if (ops.length > 0) ctx.dispatchIntent(ops);
  });
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>
</section>
