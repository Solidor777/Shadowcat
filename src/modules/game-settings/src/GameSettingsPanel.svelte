<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc,
    DEFAULT_WORLD_SETTINGS,
    type WorldSettingsSystem, type LightGradationSystem, type VisionModesSystem,
    type SceneSystem, type WireDocument,
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

  // Per-scene overrides: scene list + selection + resolved system body.
  // subscribe() is called inside each $derived.by so they re-resolve when the doc store
  // updates after the resync stream lands (same reactive pattern as ws/lgDoc/vmDoc above).
  const scenes = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("scene");
  });
  let selectedSceneId = $state<string | null>(null);
  const scene = $derived.by((): WireDocument | undefined =>
    scenes.find((s) => s.id === (selectedSceneId ?? scenes[0]?.id)));
  const ssys = $derived.by((): SceneSystem | undefined => scene?.system as SceneSystem | undefined);

  // Single-field JSON-pointer update against the SELECTED scene doc.
  // INVARIANT: scene must be defined; callers guard with the {#if} block.
  function setScene(path: string, value: unknown): void {
    if (!scene) return;
    ctx.dispatchIntent([{ op: "update", doc_id: scene.id, changes: [{ path, old: null, new: value }] }]);
  }
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
            {ctx.t("gameSettings.illuminationFloor")}
            <select
              aria-label="gameSettings.visionMode.{mode.id}"
              value={mode.illuminationFloor}
              onchange={(e) => set(vmDoc.id, `/system/modes/${mode.id}/illuminationFloor`, (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each ILLUMINATION_FLOORS as f}<option value={f}>{f}</option>{/each}
            </select>
          </label>
          <label>
            {ctx.t("gameSettings.visionModeRange")}
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

  {#if ctx.role === "gm" && scene && ssys}
    <!-- Per-scene overrides: vision, lighting, and grid.distance.
         Writing null to a field is equivalent to "inherit": resolveSceneSettings reads each
         field via nullish-coalescing (v.field ?? d.scene.field), so null falls through to the
         world default. set_pointer removal is deferred; null is the correct mechanism here.
         JSON-pointer paths written to the selected scene doc (not the world-settings doc).
         INVARIANT: setScene guards scene != null; this block only renders when scene is defined. -->
    <fieldset>
      <legend>{ctx.t("gameSettings.scene.title")}</legend>

      {#if scenes.length > 1}
        <!-- Scene picker — only shown when >1 scene exists in this world. -->
        <label>
          {ctx.t("gameSettings.scene.pick")}
          <select aria-label="gameSettings.scene.pick" value={scene.id}
            onchange={(e) => (selectedSceneId = (e.currentTarget as HTMLSelectElement).value)}>
            {#each scenes as s}<option value={s.id}>{s.id}</option>{/each}
          </select>
        </label>
      {/if}

      <!-- Vision overrides: selecting the inherit option writes null so the field is cleared
           back to the world default (null ?? default → default in resolveSceneSettings). -->
      <label>
        {ctx.t("gameSettings.scene.movementRestriction")}
        <select aria-label="gameSettings.scene.movementRestriction"
          value={ssys.vision?.movementRestriction ?? ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/vision/movementRestriction", v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.losRestriction")}
        <select aria-label="gameSettings.scene.losRestriction"
          value={ssys.vision?.losRestriction == null ? "" : ssys.vision.losRestriction ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/vision/losRestriction", v === "" ? null : v === "true");
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          <option value="true">{ctx.t("gameSettings.enabled")}</option>
          <option value="false">{ctx.t("gameSettings.disabled")}</option>
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.fog")}
        <select aria-label="gameSettings.scene.fog"
          value={ssys.vision?.fog == null ? "" : ssys.vision.fog ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/vision/fog", v === "" ? null : v === "true");
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          <option value="true">{ctx.t("gameSettings.enabled")}</option>
          <option value="false">{ctx.t("gameSettings.disabled")}</option>
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.observerVision")}
        <select aria-label="gameSettings.scene.observerVision"
          value={ssys.vision?.observerVision == null ? "" : ssys.vision.observerVision ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/vision/observerVision", v === "" ? null : v === "true");
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          <option value="true">{ctx.t("gameSettings.enabled")}</option>
          <option value="false">{ctx.t("gameSettings.disabled")}</option>
        </select>
      </label>

      <!-- Lighting overrides -->
      <label>
        {ctx.t("gameSettings.scene.lightingEnabled")}
        <select aria-label="gameSettings.scene.lightingEnabled"
          value={ssys.lighting?.enabled == null ? "" : ssys.lighting.enabled ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/lighting/enabled", v === "" ? null : v === "true");
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          <option value="true">{ctx.t("gameSettings.enabled")}</option>
          <option value="false">{ctx.t("gameSettings.disabled")}</option>
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.lightMode")}
        <select aria-label="gameSettings.scene.lightMode"
          value={ssys.lighting?.mode ?? ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/system/lighting/mode", v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>

      <!-- Environment lighting override: a tri-state select gates the color+intensity inputs.
           Selecting "inherit" writes null to /system/lighting/environment so the nullish-coalesce
           in resolveSceneSettings falls back to the world default (null ?? d.scene.environment).
           Selecting "override" seeds with DEFAULT_WORLD_SETTINGS.scene.environment so the initial
           write has a meaningful value, not #000000/0. The object is cloned (not passed by ref)
           because DEFAULT_WORLD_SETTINGS is deep-frozen. -->
      <label>
        {ctx.t("gameSettings.scene.environment")}
        <select aria-label="gameSettings.scene.environment"
          value={ssys.lighting?.environment != null ? "override" : ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            if (v === "") {
              setScene("/system/lighting/environment", null);
            } else {
              // Seed from the current override if present; fall back to the built-in default
              // (cloned — DEFAULT_WORLD_SETTINGS is deep-frozen and must not be dispatched by ref).
              setScene("/system/lighting/environment", ssys?.lighting?.environment != null
                ? { ...ssys.lighting.environment }
                : { ...DEFAULT_WORLD_SETTINGS.scene.environment });
            }
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          <option value="override">{ctx.t("gameSettings.enabled")}</option>
        </select>
      </label>

      {#if ssys.lighting?.environment != null}
        <label>
          {ctx.t("gameSettings.scene.envColor")}
          <input type="color" aria-label="gameSettings.scene.envColor"
            value={ssys.lighting.environment.color}
            onchange={(e) => {
              // Coupling: reads sibling intensity from the current override (always present in
              // this branch) to avoid overwriting it with a stale value.
              setScene("/system/lighting/environment", {
                color: (e.currentTarget as HTMLInputElement).value,
                intensity: ssys!.lighting!.environment!.intensity,
              });
            }} />
        </label>

        <label>
          {ctx.t("gameSettings.scene.envIntensity")}
          <!-- Blank ("") intensity means "environment absent / inherit"; intensity 0 is a real value. -->
          <input type="number" min="0" max="1" step="0.05" aria-label="gameSettings.scene.envIntensity"
            value={ssys.lighting.environment.intensity}
            onchange={(e) => {
              // Coupling: reads sibling color from the current override (always present in
              // this branch) to avoid overwriting it with a stale value.
              setScene("/system/lighting/environment", {
                color: ssys!.lighting!.environment!.color,
                intensity: Number((e.currentTarget as HTMLInputElement).value),
              });
            }} />
        </label>
      {/if}

      <!-- Grid distance override: un-edited sibling is read from the current override when
           present, or falls back to the defaults that resolveSceneSettings uses (5 ft/cell). -->
      <label>
        {ctx.t("gameSettings.scene.distancePerCell")}
        <input type="number" min="0" step="0.5" aria-label="gameSettings.scene.distancePerCell"
          value={ssys.grid?.distance?.perCell ?? ""}
          onchange={(e) => setScene("/system/grid/distance", {
            perCell: Number((e.currentTarget as HTMLInputElement).value),
            unit: ssys?.grid?.distance?.unit ?? "ft",
          })} />
      </label>

      <label>
        {ctx.t("gameSettings.scene.distanceUnit")}
        <input type="text" aria-label="gameSettings.scene.distanceUnit"
          value={ssys.grid?.distance?.unit ?? ""}
          onchange={(e) => setScene("/system/grid/distance", {
            perCell: ssys?.grid?.distance?.perCell ?? 5,
            unit: (e.currentTarget as HTMLInputElement).value,
          })} />
      </label>
    </fieldset>
  {/if}
</section>
