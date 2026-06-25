<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc,
    type WorldSettingsSystem, type LightGradationSystem, type VisionModesSystem, type WireDocument,
  } from "@shadowcat/core";

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

  // Derived reads — each calls subscribe() so they re-resolve when the doc store updates
  // (reactive subscription pattern; matches FactionsPanel's registry/$factionEntries deriveds).
  const ws = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const wsys = $derived.by((): WorldSettingsSystem | undefined => ws?.system as WorldSettingsSystem | undefined);

  const lgDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("light-gradation")[0];
  });
  const lgsys = $derived.by((): LightGradationSystem | undefined => lgDoc?.system as LightGradationSystem | undefined);

  const vmDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("vision-modes")[0];
  });
  const vmsys = $derived.by((): VisionModesSystem | undefined => vmDoc?.system as VisionModesSystem | undefined);

  // Single-field JSON-pointer update against a config doc.
  // INVARIANT: doc must be defined; callers guard with the {#if} block.
  function set(docId: string, path: string, value: unknown): void {
    ctx.dispatchIntent([{ op: "update", doc_id: docId, changes: [{ path, old: null, new: value }] }]);
  }

  const MOVEMENT = ["visible", "revealed", "unrestricted"] as const;
  const LIGHTMODE = ["environmentLight", "globalIllumination"] as const;
  const DIAGONAL = ["chebyshev", "alternating", "euclidean", "manhattan"] as const;
  const EASING = ["easeInOut", "linear"] as const;
  // Gradation band illumination floors as strings for the select control.
  const ILLUMINATION_FLOORS = ["bright", "dim", "dark"] as const;
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>

  {#if ctx.role === "gm" && wsys && ws}
    <!-- World-defaults: movement, lighting, light mode, pathfinding, animation -->
    <label>
      {ctx.t("gameSettings.movementRestriction")}
      <select aria-label="gameSettings.movementRestriction" value={wsys.scene.movementRestriction}
        onchange={(e) => set(ws.id, "/system/scene/movementRestriction", (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.lightingEnabled")}
      <input type="checkbox" aria-label="gameSettings.lightingEnabled" checked={wsys.scene.lightingEnabled}
        onchange={(e) => set(ws.id, "/system/scene/lightingEnabled", (e.currentTarget as HTMLInputElement).checked)} />
    </label>

    <label>
      {ctx.t("gameSettings.lightMode")}
      <select aria-label="gameSettings.lightMode" value={wsys.scene.lightMode}
        onchange={(e) => set(ws.id, "/system/scene/lightMode", (e.currentTarget as HTMLSelectElement).value)}>
        {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.diagonalRule")}
      <select aria-label="gameSettings.diagonalRule" value={wsys.pathfinding.diagonalRule}
        onchange={(e) => set(ws.id, "/system/pathfinding/diagonalRule", (e.currentTarget as HTMLSelectElement).value)}>
        {#each DIAGONAL as d}<option value={d}>{d}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.animSpeed")}
      <input type="number" min="1" step="1" aria-label="gameSettings.animSpeed" value={wsys.animation.speedCellsPerSec}
        onchange={(e) => set(ws.id, "/system/animation/speedCellsPerSec", Number((e.currentTarget as HTMLInputElement).value))} />
    </label>

    <label>
      {ctx.t("gameSettings.animEasing")}
      <select aria-label="gameSettings.animEasing" value={wsys.animation.easing}
        onchange={(e) => set(ws.id, "/system/animation/easing", (e.currentTarget as HTMLSelectElement).value)}>
        {#each EASING as ea}<option value={ea}>{ea}</option>{/each}
      </select>
    </label>
  {/if}

  {#if ctx.role === "gm" && lgsys && lgDoc}
    <!-- Gradation band editors: one numeric threshold input per seeded band.
         JSON-pointer path: /system/bands/<i>/minIllumination -->
    <fieldset>
      <legend>{ctx.t("gameSettings.gradation")}</legend>
      {#each lgsys.bands as band, i (band.name)}
        <label>
          {band.name}
          <input
            type="number" min="0" max="1" step="0.01"
            aria-label="gameSettings.gradation.{band.name}"
            value={band.minIllumination}
            onchange={(e) => set(lgDoc.id, `/system/bands/${i}/minIllumination`, Number((e.currentTarget as HTMLInputElement).value))}
          />
        </label>
      {/each}
    </fieldset>
  {/if}

  {#if ctx.role === "gm" && vmsys && vmDoc}
    <!-- Vision-mode editors: one row per mode — illumination floor select + default range number.
         JSON-pointer paths: /system/modes/<id>/illuminationFloor, /system/modes/<id>/defaultRange -->
    <fieldset>
      <legend>{ctx.t("gameSettings.visionModes")}</legend>
      {#each Object.values(vmsys.modes) as mode (mode.id)}
        <div>
          <span>{mode.name}</span>
          <label>
            {ctx.t("gameSettings.visionModes")}
            <select
              aria-label="gameSettings.visionMode.{mode.id}"
              value={mode.illuminationFloor}
              onchange={(e) => set(vmDoc.id, `/system/modes/${mode.id}/illuminationFloor`, (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each ILLUMINATION_FLOORS as f}<option value={f}>{f}</option>{/each}
            </select>
          </label>
          <label>
            {ctx.t("gameSettings.animSpeed")}
            <input
              type="number" min="0" step="1"
              aria-label="gameSettings.visionMode.{mode.id}.range"
              value={mode.defaultRange}
              onchange={(e) => set(vmDoc.id, `/system/modes/${mode.id}/defaultRange`, Number((e.currentTarget as HTMLInputElement).value))}
            />
          </label>
        </div>
      {/each}
    </fieldset>
  {/if}
</section>
