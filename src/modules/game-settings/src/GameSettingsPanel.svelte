<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    buildWorldSettingsDoc, buildLightGradationDoc, buildVisionModesDoc, buildDiceSettingsDoc,
    buildChatSettingsDoc,
    DEFAULT_WORLD_SETTINGS,
    type WorldSettingsEngine, type LightGradationEngine, type VisionModesEngine,
    type SceneEngine, type WireDocument, DEFAULT_SCENE_BOUNDS, type DiceSettingsEngine,
    type ChatSettingsEngine, type ChannelRegistryEngine,
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
    if (ctx.documents.query("dice-settings").length === 0) {
      ops.push({ op: "create" as const, doc: buildDiceSettingsDoc(ctx.world, { mode: "total", direction: "high_wins", channel_overrides: {} }) });
    }
    // chat-settings mirrors the server's ChatContentPolicy::default() (every toggle
    // off/absent — plain text, previews resolve false since hyperlinks is off).
    if (ctx.documents.query("chat-settings").length === 0) {
      ops.push({
        op: "create" as const,
        doc: buildChatSettingsDoc(ctx.world, {
          markdown: null, html: null, images: null, hyperlinks: null, emails: null, link_previews: null,
        }),
      });
    }
    seeded = true;
    if (ops.length > 0) ctx.dispatchIntent(ops);
  });

  // Derived reads — each calls subscribe() so they re-resolve when the doc store updates
  // (reactive subscription pattern; matches FactionsPanel's registry/$factionEntries deriveds).
  const ws = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const wsys = $derived.by((): WorldSettingsEngine | undefined => ws?.engine as WorldSettingsEngine | undefined);

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
   * if (ws && wsys) set(ws.id, "/engine/scene/movementRestriction", wsys.scene.movementRestriction, "visible");
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
  // Gradation band illumination floors as strings for the select control.
  const ILLUMINATION_FLOORS = ["bright", "dim", "dark"] as const;
  const DICE_MODE = ["total", "success_count"] as const;
  const DICE_DIRECTION = ["high_wins", "low_wins"] as const;

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
</script>

<section aria-label={ctx.t("gameSettings.title")}>
  <h2>{ctx.t("gameSettings.title")}</h2>

  {#if ctx.role === "gm" && wsys && ws}
    <!-- World-defaults: movement, lighting, light mode, pathfinding, animation -->
    <label>
      {ctx.t("gameSettings.movementRestriction")}
      <select aria-label="gameSettings.movementRestriction" value={wsys.scene.movementRestriction}
        onchange={(e) => set(ws.id, "/engine/scene/movementRestriction", wsys.scene.movementRestriction, (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.movementModel")}
      <select aria-label="gameSettings.movementModel" value={wsys.scene.movementModel}
        onchange={(e) => set(ws.id, "/engine/scene/movementModel", wsys.scene.movementModel, (e.currentTarget as HTMLSelectElement).value)}>
        {#each MOVEMENT_MODEL as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.lightingEnabled")}
      <input type="checkbox" aria-label="gameSettings.lightingEnabled" checked={wsys.scene.lightingEnabled}
        onchange={(e) => set(ws.id, "/engine/scene/lightingEnabled", wsys.scene.lightingEnabled, (e.currentTarget as HTMLInputElement).checked)} />
    </label>

    <label>
      {ctx.t("gameSettings.lightMode")}
      <select aria-label="gameSettings.lightMode" value={wsys.scene.lightMode}
        onchange={(e) => set(ws.id, "/engine/scene/lightMode", wsys.scene.lightMode, (e.currentTarget as HTMLSelectElement).value)}>
        {#each LIGHTMODE as m}<option value={m}>{m}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.diagonalRule")}
      <select aria-label="gameSettings.diagonalRule" value={wsys.pathfinding.diagonalRule}
        onchange={(e) => set(ws.id, "/engine/pathfinding/diagonalRule", wsys.pathfinding.diagonalRule, (e.currentTarget as HTMLSelectElement).value)}>
        {#each DIAGONAL as d}<option value={d}>{d}</option>{/each}
      </select>
    </label>

    <label>
      {ctx.t("gameSettings.animSpeed")}
      <input type="number" min="1" step="1" aria-label="gameSettings.animSpeed" value={wsys.animation.speedCellsPerSec}
        onchange={(e) => set(ws.id, "/engine/animation/speedCellsPerSec", wsys.animation.speedCellsPerSec, Number((e.currentTarget as HTMLInputElement).value))} />
    </label>

    <label>
      {ctx.t("gameSettings.animEasing")}
      <select aria-label="gameSettings.animEasing" value={wsys.animation.easing}
        onchange={(e) => set(ws.id, "/engine/animation/easing", wsys.animation.easing, (e.currentTarget as HTMLSelectElement).value)}>
        {#each EASING as ea}<option value={ea}>{ea}</option>{/each}
      </select>
    </label>
  {/if}

  {#if ctx.role === "gm" && lgsys && lgDoc}
    <!-- Gradation band editors: one numeric threshold input per seeded band.
         JSON-pointer path: /engine/bands/<i>/minIllumination -->
    <fieldset>
      <legend>{ctx.t("gameSettings.gradation")}</legend>
      {#each lgsys.bands as band, i (band.name)}
        <label>
          {band.name}
          <input
            type="number" min="0" max="1" step="0.01"
            aria-label="gameSettings.gradation.{band.name}"
            value={band.minIllumination}
            onchange={(e) => set(lgDoc.id, `/engine/bands/${i}/minIllumination`, band.minIllumination, Number((e.currentTarget as HTMLInputElement).value))}
          />
        </label>
      {/each}
    </fieldset>
  {/if}

  {#if ctx.role === "gm" && vmsys && vmDoc}
    <!-- Vision-mode editors: one row per mode — illumination floor select + default range number.
         JSON-pointer paths: /engine/modes/<id>/illuminationFloor, /engine/modes/<id>/defaultRange -->
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
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/illuminationFloor`, mode.illuminationFloor, (e.currentTarget as HTMLSelectElement).value)}
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
              onchange={(e) => set(vmDoc.id, `/engine/modes/${mode.id}/defaultRange`, mode.defaultRange, Number((e.currentTarget as HTMLInputElement).value))}
            />
          </label>
        </div>
      {/each}
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
  input {
    @media (pointer: coarse) {
      min-height: var(--input-height-coarse);
    }
  }
</style>
