<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    DEFAULT_WORLD_SETTINGS,
    resolveSettingProvenance,
    resolveGradation,
    type WorldSettingsEngine, type LightGradationEngine, type VisionModesEngine, type VisionMode, type Perception,
    type SceneEngine, type WireDocument, DEFAULT_SCENE_BOUNDS, type DiceSettingsEngine,
    type ChatSettingsEngine, type ChannelRegistryEngine,
    type SettingPath,
  } from "@shadowcat/core";

  const ctx = getAppContext();

  // Reactive subscription: calling subscribe() inside a $derived/$effect registers a reactive
  // dependency on the document store so reads re-evaluate after the resync stream populates it
  // post-mount. This panel only EDITS config singletons — the server seeds every one of them at
  // world creation and world join, so no client-side create path exists here.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // Derived reads — each calls subscribe() so they re-resolve when the doc store updates
  // (reactive subscription pattern; matches FactionsPanel's registry/$factionEntries deriveds).
  const ws = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const wsys = $derived.by((): WorldSettingsEngine | undefined => ws?.engine as WorldSettingsEngine | undefined);

  /**
   * Resolve a world-defaults setting's provenance (which layer supplies its effective value).
   * `scene` is always `undefined` here: this panel's world-defaults section shows the
   * WORLD-scoped resolution (engine < system < world), never a per-scene override.
   * Calls `subscribe()` before reading the document store: `resolveSettingProvenance` reads
   * `ctx.documents` directly, and a plain method call establishes no reactive dependency in
   * Svelte 5's runes system — every other document-store read in this component goes through
   * this same bridge (see `ws` above), and this is the one call site that read the store
   * without it.
   * @param path The setting to resolve; see `SettingPath`.
   * @returns The resolved value, the layer that supplied it, and the `systemOrEngine`
   * reset-to-system baseline.
   * @example
   * ```
   * // private helper; not part of the public API
   * prov("pathfinding.diagonalRule").source; // "world" | "system" | "engine"
   * ```
   */
  function prov(path: SettingPath): {
    /** The resolved value. */
    value: unknown;
    /** The layer that supplied it. */
    source: "engine" | "system" | "world" | "scene";
    /** The reset-to-system write target (system overlay, else the built-in engine default). */
    systemOrEngine: {
      /** The resolved value. */
      value: unknown;
      /** The layer that supplied it. */
      source: "engine" | "system";
    };
  } {
    subscribe();
    return resolveSettingProvenance(ctx.documents, undefined, path);
  }

  /** Exact `WorldSceneDefaults`/`Pathfinding`/`AnimationSettings` leaves this panel's
   * world-defaults section renders a reset control for. The world layer is an
   * `Option`-lifted overlay, so reset is a CLEAR: it writes `null` at the leaf
   * (null and absent are wire-equivalent) and resolution falls through to the
   * system layer, then the engine literal — the client never writes a
   * resolved literal it would have to know. */
  type WorldDefaultsPath = Exclude<SettingPath, `combat.${string}`>;

  /**
   * Clear a world-defaults overlay leaf: writes `null` at the leaf pointer so
   * resolution falls through to the system-or-engine baseline. `old` is the
   * field's real current stored value on the world-settings doc (the OCC
   * pre-image).
   * @param path The world-defaults setting to clear.
   * @param old The field's real current stored value on the world-settings doc.
   * @example
   * ```
   * // private function; not part of the public API — wired to each reset button's onclick
   * if (ws) resetToSystem("scene.fog", wsys?.scene?.fog);
   * ```
   */
  function resetToSystem(path: WorldDefaultsPath, old: unknown): void {
    if (!ws) return;
    set(ws.id, "/engine/" + path.replace(".", "/"), old, null);
  }

  const lgDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("light-gradation")[0];
  });
  const lgsys = $derived.by((): LightGradationEngine | undefined => lgDoc?.engine as LightGradationEngine | undefined);

  const vmDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("vision-modes")[0];
  });
  const vmsys = $derived.by((): VisionModesEngine | undefined => vmDoc?.engine as VisionModesEngine | undefined);

  const diceDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("dice-settings")[0];
  });
  const dicesys = $derived.by((): DiceSettingsEngine | undefined => diceDoc?.engine as DiceSettingsEngine | undefined);

  /** One `channel-registry` entry — `ChannelRegistryEngine.channels`'s value type, named here
   * since `@shadowcat/core` doesn't re-export the generated `Channel` type on its own. */
  type ChannelEntry = ChannelRegistryEngine["channels"][string];

  // Read-only: this panel enumerates channel-registry's channels for the
  // per-channel dice editor below but never creates/edits the registry
  // itself (the chat module owns that seed/CRUD).
  const channelRegDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("channel-registry")[0];
  });
  const channelEntries = $derived.by((): [string, ChannelEntry][] => {
    const sys = channelRegDoc?.engine as ChannelRegistryEngine | undefined;
    return Object.entries(sys?.channels ?? {});
  });

  const chatDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("chat-settings")[0];
  });
  const chatsys = $derived.by((): ChatSettingsEngine | undefined => chatDoc?.engine as ChatSettingsEngine | undefined);

  /**
   * Single-field JSON-pointer update against a config doc.
   * INVARIANT: doc must be defined; callers guard with the {#if} block.
   * `old` must be the field's REAL current value (or null when genuinely absent): the server's
   * apply_intent enforces field-level OCC (actual != change.old -> Conflict), so a hardcoded
   * `old: null` is only valid once and is rejected+rolled-back on every subsequent edit once the
   * field holds a non-null value. Callers pass the value read from the panel's reactive derived
   * system object (`sys.field ?? null`), mirroring scene-tools' snap-toggle fix.
   * @param docId The config document's id to update.
   * @param path The field's JSON-pointer path within the document.
   * @param old The field's real current value (OCC pre-image), or `null`/`undefined` when
   * genuinely absent.
   * @param value The new value to write.
   * @example
   * ```
   * // private function; not part of the public API — wired to each control's
   * // onchange below
   * if (ws && wsys) set(ws.id, "/engine/scene/movementRestriction", wsys.scene?.movementRestriction, "visible");
   * ```
   */
  function set(docId: string, path: string, old: unknown, value: unknown): void {
    ctx.dispatchIntent([{ op: "update", doc_id: docId, changes: [{ path, old: old ?? null, new: value }] }]);
  }

  const MOVEMENT = ["visible", "revealed", "unrestricted"] as const;
  const GRID_KIND = ["square", "hex"] as const;
  const MOVEMENT_MODEL = ["grid-stepped", "continuous"] as const;
  const LIGHTMODE = ["environmentLight", "globalIllumination"] as const;
  const DIAGONAL = ["chebyshev", "alternating", "euclidean", "manhattan"] as const;
  const EASING = ["easeInOut", "linear"] as const;
  const PERCEPTIONS: Perception[] = ["terrain", "creatures"];
  const DICE_MODE = ["total", "success_count"] as const;
  const DICE_DIRECTION = ["high_wins", "low_wins"] as const;

  // The vision-mode floor dropdown derives from the RESOLVED gradation (the same read the
  // server's floor resolution applies), never a hardcoded band list — a band add/remove in the
  // gradation editor below is immediately visible here.
  const floorOptions = $derived.by((): string[] => {
    subscribe();
    return resolveGradation(ctx.documents).map((b) => b.name);
  });

  // Per-scene overrides: scene list + selection + resolved system body.
  // subscribe() is called inside each $derived.by so they re-resolve when the doc store
  // updates after the resync stream lands (same reactive pattern as ws/lgDoc/vmDoc above).
  const scenes = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("scene");
  });
  let selectedSceneId = $state<string | null>(null);

  // Deep-link from the scene browser's "Configure": adopt its focused scene. Only reacts to
  // a non-null change, so a manual picker change afterward is preserved until the browser re-focuses.
  $effect(() => {
    const focus = ctx.sceneSelection.configureSceneId;
    if (focus) selectedSceneId = focus;
  });

  const scene = $derived.by((): WireDocument | undefined =>
    scenes.find((s) => s.id === (selectedSceneId ?? scenes[0]?.id)));
  const ssys = $derived.by((): SceneEngine | undefined => scene?.engine as SceneEngine | undefined);

  /**
   * Single-field JSON-pointer update against the SELECTED scene doc.
   * INVARIANT: scene must be defined; callers guard with the {#if} block.
   * `old` must be the field's real current value (see `set` above for the OCC rationale).
   * @param path The field's JSON-pointer path within the selected scene document.
   * @param old The field's real current value (OCC pre-image), or `null`/`undefined` when
   * genuinely absent.
   * @param value The new value to write.
   * @example
   * ```
   * // private function; not part of the public API — wired to each per-scene
   * // control's onchange below
   * if (ssys) setScene("/engine/grid/kind", ssys.grid?.kind ?? "square", "hex");
   * ```
   */
  function setScene(path: string, old: unknown, value: unknown): void {
    if (!scene) return;
    ctx.dispatchIntent([{ op: "update", doc_id: scene.id, changes: [{ path, old: old ?? null, new: value }] }]);
  }

  /**
   * Whole-object write: set_pointer cannot create a missing /engine/bounds parent from a
   * sub-path, so we always dispatch the full { width, height } (mirrors the environment editor).
   * The unedited axis falls back to the current authored value, else DEFAULT_SCENE_BOUNDS.
   * @param axis Which bounds axis this edit changes; the other axis is carried
   * forward unchanged.
   * @param value The new value for `axis`.
   * @example
   * ```
   * // private function; not part of the public API — wired to the bounds
   * // width/height inputs below
   * setBounds("width", 4000);
   * ```
   */
  function setBounds(axis: "width" | "height", value: number): void {
    const cur = ssys?.bounds ?? DEFAULT_SCENE_BOUNDS;
    setScene("/engine/bounds", ssys?.bounds ?? null, { ...cur, [axis]: value });
  }

  /**
   * Append a gradation band with a unique default name. Whole-array write at `/engine/bands`
   * (the array is one value; a positional insert would depend on nested-pointer semantics the
   * whole-array replace sidesteps). `old` is the raw stored (unsorted) band array.
   * @example
   * ```
   * // private function; not part of the public API — wired to the gradation add button
   * addBand();
   * ```
   */
  function addBand(): void {
    if (!lgDoc || !lgsys) return;
    let n = 1;
    while (lgsys.bands.some((b) => b.name === `band-${n}`)) n++;
    set(lgDoc.id, "/engine/bands", lgsys.bands, [...lgsys.bands, { name: `band-${n}`, minIllumination: 0.5 }]);
  }

  /**
   * Remove the gradation band at stored-array index `i` (whole-array replace, same rationale
   * as `addBand`). Band NAMES are the reference key for `VisionMode.illuminationFloor`, so
   * bands are deliberately not renamable here — a silent rename would strand every mode floor
   * pointing at the old name; removing a referenced band leaves the floor select showing the
   * raw stored name (fail-visible, never silently retargeted).
   * @param i The band's index in the raw stored (unsorted) array.
   * @example
   * ```
   * // private function; not part of the public API — wired to each band's remove button
   * removeBand(0);
   * ```
   */
  function removeBand(i: number): void {
    if (!lgDoc || !lgsys) return;
    set(lgDoc.id, "/engine/bands", lgsys.bands, lgsys.bands.filter((_, j) => j !== i));
  }

  /**
   * Add a vision mode under a fresh `custom-N` id (the map key is the stable id that
   * `VisionAssignment.mode` references; the display name starts equal to the id and is
   * editable in the row). The new mode seeds as a terrain sense whose floor is the DARKEST
   * resolved band (`resolveGradation` sorts brightest-first). Written as a single-key create
   * at `/engine/modes/<id>` (`old: null`), the same set_pointer map-creation the dice
   * channel-override editor uses.
   * @example
   * ```
   * // private function; not part of the public API — wired to the vision-mode add button
   * addVisionMode();
   * ```
   */
  function addVisionMode(): void {
    if (!vmDoc || !vmsys) return;
    let n = 1;
    while (vmsys.modes[`custom-${n}`]) n++;
    const id = `custom-${n}`;
    const mode: VisionMode = {
      id,
      name: id,
      illuminationFloor: floorOptions[floorOptions.length - 1] ?? "dark",
      defaultRange: 12,
      perceives: "terrain",
      requiresLos: true,
      renderHint: null,
    };
    set(vmDoc.id, `/engine/modes/${id}`, null, mode);
  }

  /**
   * Remove a vision mode by id: whole-map replace at `/engine/modes` minus the key (the same
   * removal shape the dice channel-override editor uses — set_pointer has no map-key removal).
   * Assignments referencing the removed id dangle harmlessly: the sense resolver ignores an
   * unknown mode id, and the assignment editors show the raw id (fail-visible).
   * @param id The mode's registry id (its `modes` map key).
   * @example
   * ```
   * // private function; not part of the public API — wired to each mode's remove button
   * removeVisionMode("tremorsense");
   * ```
   */
  function removeVisionMode(id: string): void {
    if (!vmDoc || !vmsys) return;
    const next = { ...vmsys.modes };
    delete next[id];
    set(vmDoc.id, "/engine/modes", vmsys.modes, next);
  }

  /**
   * Commit a mode's display name edit; a blank name is ignored (a nameless row would be
   * unidentifiable in every select that lists modes by name).
   * @param id The mode's registry id.
   * @param raw The name input's raw string value.
   * @example
   * ```
   * // private function; not part of the public API — wired to each mode's name input
   * commitModeName("darkvision", "Darkvision (60 ft)");
   * ```
   */
  function commitModeName(id: string, raw: string): void {
    const mode = vmsys?.modes[id];
    if (!vmDoc || !mode) return;
    const name = raw.trim();
    if (name === "" || name === mode.name) return;
    set(vmDoc.id, `/engine/modes/${id}/name`, mode.name, name);
  }

  /**
   * Commit a mode's render-hint edit; an emptied input writes `null` (the field is
   * `string | null`, absent = no render treatment).
   * @param id The mode's registry id.
   * @param raw The render-hint input's raw string value.
   * @example
   * ```
   * // private function; not part of the public API — wired to each mode's render-hint input
   * commitModeRenderHint("darkvision", "desaturate");
   * ```
   */
  function commitModeRenderHint(id: string, raw: string): void {
    const mode = vmsys?.modes[id];
    if (!vmDoc || !mode) return;
    const hint = raw.trim();
    const next = hint === "" ? null : hint;
    if (next === mode.renderHint) return;
    set(vmDoc.id, `/engine/modes/${id}/renderHint`, mode.renderHint ?? null, next);
  }
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>

  <!-- Per-control provenance hint + reset-to-system-default button, shared by every world-defaults
       control below. Provenance is structural on the overlay: a PRESENT world leaf IS an
       override, so the reset button renders exactly when a stored leaf exists to clear. -->
  {#snippet provControl(path: WorldDefaultsPath, old: unknown)}
    {@const p = prov(path)}
    <p class="hint" data-testid={"provenance:" + path}>{ctx.t("gameSettings.source." + p.source)}</p>
    {#if p.source === "world"}
      <button type="button" class="reset-to-system" aria-label={"gameSettings.resetToSystem:" + path}
        onclick={() => resetToSystem(path, old)}>{ctx.t("gameSettings.resetToSystem")}</button>
    {/if}
  {/snippet}

  {#if ctx.role === "gm" && wsys && ws}
    <!-- World-defaults: movement, lighting, light mode, fog, pathfinding, animation -->
    <label>
      {ctx.t("gameSettings.movementRestriction")}
      <select aria-label="gameSettings.movementRestriction" value={prov("scene.movementRestriction").value as string}
        onchange={(e) => set(ws.id, "/engine/scene/movementRestriction", wsys.scene?.movementRestriction, (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>
    {@render provControl("scene.movementRestriction", wsys.scene?.movementRestriction)}

    <label>
      {ctx.t("gameSettings.movementModel")}
      <select aria-label="gameSettings.movementModel" value={prov("scene.movementModel").value as string}
        onchange={(e) => set(ws.id, "/engine/scene/movementModel", wsys.scene?.movementModel, (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT_MODEL as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>
    {@render provControl("scene.movementModel", wsys.scene?.movementModel)}

    <label>
      {ctx.t("gameSettings.lightingEnabled")}
      <input type="checkbox" aria-label="gameSettings.lightingEnabled" checked={prov("scene.lightingEnabled").value === true}
        onchange={(e) => set(ws.id, "/engine/scene/lightingEnabled", wsys.scene?.lightingEnabled, (e.currentTarget as HTMLInputElement).checked)} />
    </label>
    {@render provControl("scene.lightingEnabled", wsys.scene?.lightingEnabled)}

    <label>
      {ctx.t("gameSettings.lightMode")}
      <select aria-label="gameSettings.lightMode" value={prov("scene.lightMode").value as string}
        onchange={(e) => set(ws.id, "/engine/scene/lightMode", wsys.scene?.lightMode, (e.currentTarget as HTMLSelectElement).value)}>
        {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>
    {@render provControl("scene.lightMode", wsys.scene?.lightMode)}

    <!-- No editable world-level fog control exists: only the per-scene override below has an
         input. This section renders the provenance hint + reset-to-system button standalone,
         reading the current stored value from wsys for the reset's OCC pre-image. -->
    {@render provControl("scene.fog", wsys.scene?.fog)}

    <label>
      {ctx.t("gameSettings.diagonalRule")}
      <select aria-label="gameSettings.diagonalRule" value={prov("pathfinding.diagonalRule").value as string}
        onchange={(e) => set(ws.id, "/engine/pathfinding/diagonalRule", wsys.pathfinding?.diagonalRule, (e.currentTarget as HTMLSelectElement).value)}>
        {#each DIAGONAL as d}<option value={d}>{d}</option>{/each}
      </select>
    </label>
    {@render provControl("pathfinding.diagonalRule", wsys.pathfinding?.diagonalRule)}

    <label>
      {ctx.t("gameSettings.animSpeed")}
      <input type="number" min="1" step="1" aria-label="gameSettings.animSpeed" value={prov("animation.speedCellsPerSec").value as number}
        onchange={(e) => set(ws.id, "/engine/animation/speedCellsPerSec", wsys.animation?.speedCellsPerSec, Number((e.currentTarget as HTMLInputElement).value))} />
    </label>
    {@render provControl("animation.speedCellsPerSec", wsys.animation?.speedCellsPerSec)}

    <label>
      {ctx.t("gameSettings.animEasing")}
      <select aria-label="gameSettings.animEasing" value={prov("animation.easing").value as string}
        onchange={(e) => set(ws.id, "/engine/animation/easing", wsys.animation?.easing, (e.currentTarget as HTMLSelectElement).value)}>
        {#each EASING as ea}<option value={ea}>{ea}</option>{/each}
      </select>
    </label>
    {@render provControl("animation.easing", wsys.animation?.easing)}
  {/if}

  {#if ctx.role === "gm" && lgsys && lgDoc}
    <!-- Gradation band editors: one numeric threshold input + one remove button per band.
         JSON-pointer paths: /engine/bands/<i>/minIllumination; add/remove write the WHOLE
         /engine/bands array. Band names are reference keys (VisionMode.illuminationFloor
         names a band), so they are displayed but never editable here. -->
    <fieldset>
      <legend>{ctx.t("gameSettings.gradation")}</legend>
      {#each lgsys.bands as band, i (band.name)}
        <div class="band-row">
          <label>
            {band.name}
            <input
              type="number" min="0" max="1" step="0.01"
              aria-label="gameSettings.gradation.{band.name}"
              value={band.minIllumination}
              onchange={(e) => set(lgDoc.id, `/engine/bands/${i}/minIllumination`, band.minIllumination, Number((e.currentTarget as HTMLInputElement).value))}
            />
          </label>
          <button type="button" aria-label="gameSettings.gradationRemove.{band.name}"
            onclick={() => removeBand(i)}>{ctx.t("gameSettings.gradationRemove")}</button>
        </div>
      {/each}
      <button type="button" aria-label="gameSettings.gradationAdd"
        onclick={addBand}>{ctx.t("gameSettings.gradationAdd")}</button>
    </fieldset>
  {/if}

  {#if ctx.role === "gm" && vmsys && vmDoc}
    <!-- Vision-mode editors: one row per mode — name, what it perceives, LOS requirement,
         render hint, illumination floor (options derive from the resolved gradation), default
         range, remove. The add button creates a `custom-N` mode.
         JSON-pointer paths: /engine/modes/<id>/<field>; removal replaces the whole
         /engine/modes map. The mode id is the map key and never editable (assignments
         reference it); the display name is. -->
    <fieldset>
      <legend>{ctx.t("gameSettings.visionModes")}</legend>
      {#each Object.values(vmsys.modes) as mode (mode.id)}
        <div class="mode-row">
          <label>
            {ctx.t("gameSettings.visionModeName")}
            <input
              type="text"
              aria-label="gameSettings.visionMode.{mode.id}.name"
              value={mode.name}
              onchange={(e) => commitModeName(mode.id, (e.currentTarget as HTMLInputElement).value)}
            />
          </label>
          <label>
            {ctx.t("gameSettings.visionModePerceives")}
            <select
              aria-label="gameSettings.visionMode.{mode.id}.perceives"
              value={mode.perceives}
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/perceives`, mode.perceives, (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each PERCEPTIONS as p}<option value={p}>{p === "terrain" ? ctx.t("gameSettings.perceivesTerrain") : ctx.t("gameSettings.perceivesCreatures")}</option>{/each}
            </select>
          </label>
          <label>
            <input
              type="checkbox"
              aria-label="gameSettings.visionMode.{mode.id}.requiresLos"
              checked={mode.requiresLos}
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/requiresLos`, mode.requiresLos, (e.currentTarget as HTMLInputElement).checked)}
            />
            {ctx.t("gameSettings.visionModeRequiresLos")}
          </label>
          <label>
            {ctx.t("gameSettings.visionModeRenderHint")}
            <input
              type="text"
              aria-label="gameSettings.visionMode.{mode.id}.renderHint"
              value={mode.renderHint ?? ""}
              onchange={(e) => commitModeRenderHint(mode.id, (e.currentTarget as HTMLInputElement).value)}
            />
          </label>
          <label>
            {ctx.t("gameSettings.illuminationFloor")}
            <select
              aria-label="gameSettings.visionMode.{mode.id}"
              value={mode.illuminationFloor}
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/illuminationFloor`, mode.illuminationFloor, (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each floorOptions as f}<option value={f}>{f}</option>{/each}
              {#if !floorOptions.includes(mode.illuminationFloor)}
                <!-- A floor naming a removed/absent band stays visible as its raw stored
                     value rather than silently displaying a different band. -->
                <option value={mode.illuminationFloor}>{mode.illuminationFloor}</option>
              {/if}
            </select>
          </label>
          <label>
            {ctx.t("gameSettings.visionModeRange")}
            <input
              type="number" min="0" step="1"
              aria-label="gameSettings.visionMode.{mode.id}.range"
              value={mode.defaultRange}
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/defaultRange`, mode.defaultRange, Number((e.currentTarget as HTMLInputElement).value))}
            />
          </label>
          <button type="button" aria-label="gameSettings.visionModeRemove.{mode.id}"
            onclick={() => removeVisionMode(mode.id)}>{ctx.t("gameSettings.visionModeRemove")}</button>
        </div>
      {/each}
      <button type="button" aria-label="gameSettings.visionModeAdd"
        onclick={addVisionMode}>{ctx.t("gameSettings.visionModeAdd")}</button>
    </fieldset>
  {/if}

  {#if ctx.role === "gm" && dicesys && diceDoc}
    <!-- Ambient dice-notation context: world-default mode (Total/Success count) and
         direction (High/Low wins) at /engine/mode, /engine/direction, plus a per-channel
         override editor below writing /engine/channel_overrides/<id> (or a whole-map
         replace at /engine/channel_overrides to remove one). Matches the server body shape
         (`DiceSettingsEngine`) exactly: mode "total"|"success_count", direction
         "high_wins"|"low_wins", channel_overrides a map of channel id to a full-replacement
         {mode, direction} pair. -->
    <fieldset>
      <legend>{ctx.t("gameSettings.dice.title")}</legend>
      <label>
        {ctx.t("gameSettings.dice.mode")}
        <select aria-label="gameSettings.dice.mode" value={dicesys.mode}
          onchange={(e) => set(diceDoc.id, "/engine/mode", dicesys.mode, (e.currentTarget as HTMLSelectElement).value)}>
          {#each DICE_MODE as m}
            <option value={m}>{m === "total" ? ctx.t("gameSettings.dice.modeTotal") : ctx.t("gameSettings.dice.modeSuccess")}</option>
          {/each}
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.dice.direction")}
        <select aria-label="gameSettings.dice.direction" value={dicesys.direction}
          onchange={(e) => set(diceDoc.id, "/engine/direction", dicesys.direction, (e.currentTarget as HTMLSelectElement).value)}>
          {#each DICE_DIRECTION as d}
            <option value={d}>{d === "high_wins" ? ctx.t("gameSettings.dice.directionHigh") : ctx.t("gameSettings.dice.directionLow")}</option>
          {/each}
        </select>
      </label>

      {#if channelEntries.length > 0}
        {@const overrides = dicesys.channel_overrides ?? {}}
        <div>
          <span>{ctx.t("gameSettings.dice.channelOverrides")}</span>
          {#each channelEntries as [id, channel] (id)}
            {@const override = overrides[id]}
            <div>
              <span>{channel.name}</span>
              <label>
                {ctx.t("gameSettings.dice.channelOverride")}
                <select aria-label="gameSettings.dice.channelOverride.{id}"
                  value={override != null ? "override" : ""}
                  onchange={(e) => {
                    const v = (e.currentTarget as HTMLSelectElement).value;
                    if (v === "") {
                      const next = { ...overrides };
                      delete next[id];
                      set(diceDoc.id, "/engine/channel_overrides", overrides, next);
                    } else {
                      set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: dicesys.mode, direction: dicesys.direction });
                    }
                  }}>
                  <option value="">{ctx.t("gameSettings.inherit")}</option>
                  <option value="override">{ctx.t("gameSettings.dice.channelOverrideCustom")}</option>
                </select>
              </label>
              {#if override != null}
                <label>
                  {ctx.t("gameSettings.dice.mode")}
                  <select aria-label="gameSettings.dice.channelOverride.{id}.mode" value={override.mode}
                    onchange={(e) => set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: (e.currentTarget as HTMLSelectElement).value, direction: override.direction })}>
                    {#each DICE_MODE as m}
                      <option value={m}>{m === "total" ? ctx.t("gameSettings.dice.modeTotal") : ctx.t("gameSettings.dice.modeSuccess")}</option>
                    {/each}
                  </select>
                </label>
                <label>
                  {ctx.t("gameSettings.dice.direction")}
                  <select aria-label="gameSettings.dice.channelOverride.{id}.direction" value={override.direction}
                    onchange={(e) => set(diceDoc.id, `/engine/channel_overrides/${id}`, override, { mode: override.mode, direction: (e.currentTarget as HTMLSelectElement).value })}>
                    {#each DICE_DIRECTION as d}
                      <option value={d}>{d === "high_wins" ? ctx.t("gameSettings.dice.directionHigh") : ctx.t("gameSettings.dice.directionLow")}</option>
                    {/each}
                  </select>
                </label>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </fieldset>
  {/if}

  {#if ctx.role === "gm" && chatsys && chatDoc}
    <!-- Chat content policy: hyperlinks toggle + link-preview tri-state.
         JSON-pointer paths: /engine/hyperlinks, /engine/link_previews.
         Both fields are `boolean | null` on the wire (`ChatSettingsEngine`,
         aliased as `ChatContentPolicy`) — but they differ in what null MEANS, and each
         control's DISPLAY expression mirrors its server-side accessor. That
         mirroring is scoped to the read path only: the accessors resolve a
         stored value for reading, and nothing server-side ever normalizes a
         stored null to false, so an OCC `old` pre-image must still carry the
         RAW value — every control here, including hyperlinks, passes `?? null`
         as its `old` argument. hyperlinks has no inherit
         concept: `ChatContentPolicy::hyperlinks` resolves absent to false
         (`unwrap_or(false)`), so this
         panel exposes it as a plain two-state checkbox coalescing null the same
         way for DISPLAY. link_previews is genuinely TRI-STATE: `ChatContentPolicy::previews_enabled`
         is `self.hyperlinks() && self.link_previews.unwrap_or(true)`
         — absent defaults ON but only within hyperlinks-on, and true/false is
         an explicit override. That third state is why it gets a select rather
         than a checkbox; the "" option writes null, mirroring the scene-override
         inherit pattern above. -->
    <fieldset>
      <legend>{ctx.t("gameSettings.chat.title")}</legend>
      <label>
        {ctx.t("gameSettings.chat.hyperlinks")}
        <input type="checkbox" aria-label="gameSettings.chat.hyperlinks" checked={chatsys.hyperlinks ?? false}
          onchange={(e) => set(chatDoc.id, "/engine/hyperlinks", chatsys.hyperlinks ?? null, (e.currentTarget as HTMLInputElement).checked)} />
      </label>

      <label>
        {ctx.t("gameSettings.chat.linkPreviews")}
        <select aria-label="gameSettings.chat.linkPreviews"
          value={chatsys.link_previews == null ? "" : chatsys.link_previews ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            set(chatDoc.id, "/engine/link_previews", chatsys.link_previews ?? null, v === "" ? null : v === "true");
          }}>
          <option value="">{ctx.t("gameSettings.chat.linkPreviewsDefault")}</option>
          <option value="true">{ctx.t("gameSettings.enabled")}</option>
          <option value="false">{ctx.t("gameSettings.disabled")}</option>
        </select>
      </label>
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
            {#each scenes as s}<option value={s.id}>{s.name ?? s.id}</option>{/each}
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
            setScene("/engine/vision/movementRestriction", ssys.vision?.movementRestriction ?? null, v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.movementModel")}
        <select aria-label="gameSettings.scene.movementModel"
          value={ssys.vision?.movementModel ?? ""}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/engine/vision/movementModel", ssys.vision?.movementModel ?? null, v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each MOVEMENT_MODEL as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.losRestriction")}
        <select aria-label="gameSettings.scene.losRestriction"
          value={ssys.vision?.losRestriction == null ? "" : ssys.vision.losRestriction ? "true" : "false"}
          onchange={(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            setScene("/engine/vision/losRestriction", ssys.vision?.losRestriction ?? null, v === "" ? null : v === "true");
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
            setScene("/engine/vision/fog", ssys.vision?.fog ?? null, v === "" ? null : v === "true");
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
            setScene("/engine/vision/observerVision", ssys.vision?.observerVision ?? null, v === "" ? null : v === "true");
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
            setScene("/engine/lighting/enabled", ssys.lighting?.enabled ?? null, v === "" ? null : v === "true");
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
            setScene("/engine/lighting/mode", ssys.lighting?.mode ?? null, v === "" ? null : v);
          }}>
          <option value="">{ctx.t("gameSettings.inherit")}</option>
          {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>

      <!-- Environment lighting override: a tri-state select gates the color+intensity inputs.
           Selecting "inherit" writes null to /engine/lighting/environment so the nullish-coalesce
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
            const curEnv = ssys?.lighting?.environment != null ? { ...ssys.lighting.environment } : null;
            if (v === "") {
              setScene("/engine/lighting/environment", curEnv, null);
            } else {
              // Seed from the current override if present; fall back to the built-in default
              // (cloned — DEFAULT_WORLD_SETTINGS is deep-frozen and must not be dispatched by ref).
              setScene("/engine/lighting/environment", curEnv, curEnv ?? { ...DEFAULT_WORLD_SETTINGS.scene.environment });
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
              setScene("/engine/lighting/environment", { ...ssys!.lighting!.environment! }, {
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
              setScene("/engine/lighting/environment", { ...ssys!.lighting!.environment! }, {
                color: ssys!.lighting!.environment!.color,
                intensity: Number((e.currentTarget as HTMLInputElement).value),
              });
            }} />
        </label>
      {/if}

      <!-- Grid kind/size: per-scene INTRINSIC values (there is no world-level grid kind), so
           these follow the plain-value pattern (like scene bounds below), not the ""=inherit
           tri-state the vision/lighting overrides above use. -->
      <label>
        {ctx.t("gameSettings.scene.gridKind")}
        <select aria-label="gameSettings.scene.gridKind" value={ssys.grid?.kind ?? "square"}
          onchange={(e) => setScene("/engine/grid/kind", ssys.grid?.kind ?? "square", (e.currentTarget as HTMLSelectElement).value)}>
          {#each GRID_KIND as k}<option value={k}>{k}</option>{/each}
        </select>
      </label>

      <label>
        {ctx.t("gameSettings.scene.gridSize")}
        <input type="number" min="1" step="1" aria-label="gameSettings.scene.gridSize"
          value={ssys.grid?.size ?? 100}
          onchange={(e) => setScene("/engine/grid/size", ssys.grid?.size ?? 100, Number((e.currentTarget as HTMLInputElement).value))} />
      </label>

      <!-- Grid distance override: un-edited sibling is read from the current override when
           present, or falls back to the defaults that resolveSceneSettings uses (5 ft/cell). -->
      <label>
        {ctx.t("gameSettings.scene.distancePerCell")}
        <input type="number" min="0" step="0.5" aria-label="gameSettings.scene.distancePerCell"
          value={ssys.grid?.distance?.perCell ?? ""}
          onchange={(e) => setScene("/engine/grid/distance", ssys.grid?.distance ?? null, {
            perCell: Number((e.currentTarget as HTMLInputElement).value),
            unit: ssys?.grid?.distance?.unit ?? "ft",
          })} />
      </label>

      <label>
        {ctx.t("gameSettings.scene.distanceUnit")}
        <input type="text" aria-label="gameSettings.scene.distanceUnit"
          value={ssys.grid?.distance?.unit ?? ""}
          onchange={(e) => setScene("/engine/grid/distance", ssys.grid?.distance ?? null, {
            perCell: ssys?.grid?.distance?.perCell ?? 5,
            unit: (e.currentTarget as HTMLInputElement).value,
          })} />
      </label>

      <!-- Scene bounds: per-scene only, fixed default (not an inherit-from-world tri-state). -->
      <label>
        {ctx.t("gameSettings.scene.boundsWidth")}
        <input type="number" min="1" step="1" aria-label="gameSettings.scene.boundsWidth"
          value={ssys?.bounds?.width ?? DEFAULT_SCENE_BOUNDS.width}
          onchange={(e) => setBounds("width", Number((e.currentTarget as HTMLInputElement).value))} />
      </label>
      <label>
        {ctx.t("gameSettings.scene.boundsHeight")}
        <input type="number" min="1" step="1" aria-label="gameSettings.scene.boundsHeight"
          value={ssys?.bounds?.height ?? DEFAULT_SCENE_BOUNDS.height}
          onchange={(e) => setBounds("height", Number((e.currentTarget as HTMLInputElement).value))} />
      </label>
    </fieldset>
  {/if}
</section>

<style lang="scss">
  input,
  .reset-to-system {
    @media (pointer: coarse) {
      min-height: var(--input-height-coarse);
    }
  }
  .band-row,
  .mode-row {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: var(--space-1);
    align-items: end;
  }
</style>
