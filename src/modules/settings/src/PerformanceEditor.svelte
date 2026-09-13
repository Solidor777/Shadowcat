<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { PerformancePreset, PerformanceSettings } from "@shadowcat/core";

  const { t, performance } = getAppContext();

  /** The named presets offered as radios; `"custom"` is not choosable directly (it appears as
   * a badge once any field diverges — see `PerformanceController.set`). */
  const PRESET_IDS: PerformancePreset[] = ["auto", "mobile", "balanced", "quality"];
  /** Every `PerformanceSettings.fpsCap` value, in select order. */
  const FPS_CAPS: PerformanceSettings["fpsCap"][] = [30, 60, 120, "uncapped"];
  /** Every `PerformanceSettings.lighting` mode, in select order. */
  const LIGHTING_MODES: PerformanceSettings["lighting"][] = ["full", "static", "off"];

  /** The `PerformanceSettings` keys driven by a checkbox. */
  type BooleanKey = "antialias" | "tokenFx" | "vfx" | "dice3d" | "spatialAudio" | "idleSkip" | "reducedMotion";

  /** Applies the fps-cap select's value (`"uncapped"` stays the literal; digits parse).
   * @param e The select's change event.
   * @example
   * ```
   * // private function; not part of the public API — wired to the fps-cap <select>'s onchange
   * setFpsCap(new Event("change"));
   * ```
   */
  function setFpsCap(e: Event): void {
    const raw = (e.currentTarget as HTMLSelectElement).value;
    performance.set({ fpsCap: raw === "uncapped" ? "uncapped" : (Number(raw) as 30 | 60 | 120) });
  }

  /** Applies the render-scale range input's value (already bounded 0.5–1 by the input itself).
   * @param e The range input's input event.
   * @example
   * ```
   * // private function; not part of the public API — wired to the render-scale <input>'s oninput
   * setRenderScale(new Event("input"));
   * ```
   */
  function setRenderScale(e: Event): void {
    performance.set({ renderScale: Number((e.currentTarget as HTMLInputElement).value) });
  }

  /** Applies the lighting-quality select's value.
   * @param e The select's change event.
   * @example
   * ```
   * // private function; not part of the public API — wired to the lighting <select>'s onchange
   * setLighting(new Event("change"));
   * ```
   */
  function setLighting(e: Event): void {
    performance.set({ lighting: (e.currentTarget as HTMLSelectElement).value as PerformanceSettings["lighting"] });
  }

  /** Builds the change handler for one boolean budget checkbox.
   * @param key The `PerformanceSettings` boolean key the checkbox drives.
   * @returns The checkbox's change handler.
   * @example
   * ```
   * // private function; not part of the public API — wired to each boolean <input>'s onchange
   * toggle("idleSkip");
   * ```
   */
  function toggle(key: BooleanKey) {
    return (e: Event) => performance.set({ [key]: (e.currentTarget as HTMLInputElement).checked });
  }
</script>

<fieldset class="perf-editor">
  <legend>{t("performance.title")}</legend>
  <div class="preset-row" role="radiogroup" aria-label={t("performance.preset")}>
    {#each PRESET_IDS as p (p)}
      <label>
        <input
          type="radio"
          name="perf-preset"
          data-testid={`perf-preset-${p}`}
          checked={performance.preset === p}
          onchange={() => performance.setPreset(p)}
        />
        {t(`performance.preset.${p}`)}
      </label>
    {/each}
    {#if performance.preset === "custom"}
      <span class="custom-badge">{t("performance.preset.custom")}</span>
    {/if}
  </div>
  <label>{t("performance.fpsCap")}
    <select data-testid="perf-fps-cap" value={String(performance.current.fpsCap)} onchange={setFpsCap}>
      {#each FPS_CAPS as cap (cap)}
        <option value={String(cap)}>{cap === "uncapped" ? t("performance.uncapped") : cap}</option>
      {/each}
    </select>
  </label>
  <label>{t("performance.renderScale")}
    <input
      type="range"
      data-testid="perf-render-scale"
      min="0.5" max="1" step="0.05"
      value={performance.current.renderScale}
      oninput={setRenderScale}
    />
    <span>{performance.current.renderScale.toFixed(2)}</span>
  </label>
  <label>{t("performance.lighting")}
    <select data-testid="perf-lighting" value={performance.current.lighting} onchange={setLighting}>
      {#each LIGHTING_MODES as mode (mode)}
        <option value={mode}>{t(`performance.lighting.${mode}`)}</option>
      {/each}
    </select>
  </label>
  <label><input type="checkbox" data-testid="perf-antialias" checked={performance.current.antialias} onchange={toggle("antialias")} /> {t("performance.antialias")}</label>
  <label><input type="checkbox" data-testid="perf-token-fx" checked={performance.current.tokenFx} onchange={toggle("tokenFx")} /> {t("performance.tokenFx")}</label>
  <label><input type="checkbox" data-testid="perf-vfx" checked={performance.current.vfx} onchange={toggle("vfx")} /> {t("performance.vfx")}</label>
  <label><input type="checkbox" data-testid="perf-dice3d" checked={performance.current.dice3d} onchange={toggle("dice3d")} /> {t("performance.dice3d")}</label>
  <label><input type="checkbox" data-testid="perf-spatial-audio" checked={performance.current.spatialAudio} onchange={toggle("spatialAudio")} /> {t("performance.spatialAudio")}</label>
  <label><input type="checkbox" data-testid="perf-idle-skip" checked={performance.current.idleSkip} onchange={toggle("idleSkip")} /> {t("performance.idleSkip")}</label>
  <label><input type="checkbox" data-testid="perf-reduced-motion" checked={performance.current.reducedMotion} onchange={toggle("reducedMotion")} /> {t("performance.reducedMotion")}</label>
  <label><input type="checkbox" data-testid="perf-show-stats" checked={performance.showStats} onchange={(e) => performance.setShowStats((e.currentTarget as HTMLInputElement).checked)} /> {t("performance.showStats")}</label>
  <button type="button" data-testid="perf-reset" onclick={() => performance.setPreset("auto")}>{t("performance.reset")}</button>
</fieldset>

<style lang="scss">
  .perf-editor {
    display: grid;
    gap: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    padding: var(--space-3);
  }
  .preset-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  @media (pointer: coarse) {
    input[type="checkbox"],
    input[type="radio"] {
      min-width: var(--input-height-coarse);
      min-height: var(--input-height-coarse);
    }
  }
  @media (max-width: 400px) {
    .perf-editor {
      padding: var(--space-2);
    }
  }
</style>
