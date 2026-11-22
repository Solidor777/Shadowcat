<script lang="ts">
  // Shared carried-light emission editor: the field set for one `LightEmission` payload
  // (enabled / color / intensity / radii / falloff). Deliberately value-only — the OWNING
  // surface (actor row, actor sheet, token-override control) decides what `null` means around
  // it (no emission vs. inherit) and owns the dispatch, so the raw-`old` OCC pre-image read
  // stays at the call site that knows the document path.
  import { getAppContext } from "./appContext";
  import type { LightEmission, FalloffCurve } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { value, disabled = false, carriedByToken = false, onCommit }: {
    /** The emission being edited (never `null` — the parent renders this component only when an
     * emission exists and handles the absent/inherit states itself). */
    value: LightEmission;
    /** Read-only mode (a sheet the viewer may not edit). */
    disabled?: boolean;
    /** Whether the emission is carried by a token (actor row, actor sheet, token override). Only
     * then is the "follow token facing" control shown: a standalone light has no token to follow. */
    carriedByToken?: boolean;
    /** Commit callback: receives the WHOLE updated emission (whole-object replacement — the
     * emission is one nested payload, so a single `/light` write carries every field's change). */
    onCommit: (next: LightEmission) => void;
  } = $props();

  const falloffCurves: FalloffCurve[] = ["linear", "quadratic", "none"];

  /**
   * Locale label for a falloff curve option.
   * @param f The curve id.
   * @returns The localized display label.
   * @example
   * ```
   * // private helper; wired to the falloff select's options
   * falloffLabel("quadratic"); // localized "Quadratic"
   * ```
   */
  const falloffLabel = (f: FalloffCurve): string =>
    f === "linear" ? t("tools.falloffLinear") : f === "quadratic" ? t("tools.falloffQuadratic") : t("tools.falloffNone");

  /** Commit one field of the emission, preserving the rest.
   * @param patch The field(s) to change.
   * @example
   * ```
   * // private helper; wired to each control's onchange
   * commit({ intensity: 0.5 });
   * ```
   */
  function commit(patch: Partial<LightEmission>): void {
    onCommit({ ...value, ...patch });
  }

  /** Commit a numeric field from a number input, ignoring a non-finite parse.
   * @param key The emission field to write.
   * @param raw The input's raw string value.
   * @example
   * ```
   * // private helper; wired to the number inputs' onchange
   * commitNumber("brightRadius", "2.5");
   * ```
   */
  function commitNumber(key: "intensity" | "brightRadius" | "dimRadius", raw: string): void {
    // An emptied field (`Number("") === 0`, not NaN) must not silently write 0 — no commit.
    if (raw.trim() === "") return;
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    commit({ [key]: n });
  }

  /** The default darkness threshold written when "activates only in darkness" is first checked. */
  const DEFAULT_DARKNESS_THRESHOLD = 0.5;
  /** The default cone width, in degrees, written when "cone" is first checked. */
  const DEFAULT_CONE_ANGLE = 90;

  /** Toggle the darkness-activation window. Unchecking drops the field (absent = always active).
   * @param on Whether the window is enabled.
   * @example
   * ```
   * // private helper; wired to the darkness checkbox
   * setDarknessWindow(true);
   * ```
   */
  function setDarknessWindow(on: boolean): void {
    if (on) {
      commit({ activeBelowEnv: DEFAULT_DARKNESS_THRESHOLD });
      return;
    }
    const next = { ...value };
    delete next.activeBelowEnv;
    onCommit(next);
  }

  /** Commit the darkness threshold, clamped into `[0, 1]`; a non-numeric or empty edit is ignored.
   * @param raw The slider's raw string value.
   * @example
   * ```
   * // private helper; wired to the threshold slider
   * commitThreshold("1.5"); // commits 1
   * ```
   */
  function commitThreshold(raw: string): void {
    if (raw.trim() === "") return;
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    commit({ activeBelowEnv: Math.min(1, Math.max(0, n)) });
  }

  /** Toggle the cone shape. Unchecking drops the angle (absent = the full-circle radial light).
   * @param on Whether the light is a cone.
   * @example
   * ```
   * // private helper; wired to the cone checkbox
   * setCone(true);
   * ```
   */
  function setCone(on: boolean): void {
    if (on) {
      commit({ angle: DEFAULT_CONE_ANGLE });
      return;
    }
    const next = { ...value };
    delete next.angle;
    onCommit(next);
  }

  /** Commit the cone width, clamped into `(0, 360]` (the server refuses 0 and anything over a
   * full turn); a non-numeric or empty edit is ignored.
   * @param raw The input's raw string value.
   * @example
   * ```
   * // private helper; wired to the cone width input
   * commitAngle("400"); // commits 360
   * ```
   */
  function commitAngle(raw: string): void {
    if (raw.trim() === "") return;
    const n = Number(raw);
    if (!Number.isFinite(n) || n <= 0) return;
    commit({ angle: Math.min(360, n) });
  }

  /** Commit the cone direction in degrees; any finite value is legal. A non-numeric or empty
   * edit is ignored.
   * @param raw The input's raw string value.
   * @example
   * ```
   * // private helper; wired to the cone direction input
   * commitRotation("90");
   * ```
   */
  function commitRotation(raw: string): void {
    if (raw.trim() === "") return;
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    commit({ rotation: n });
  }
</script>

<div class="light-emission-editor">
  <label>
    <input
      type="checkbox"
      data-testid="emission-enabled"
      checked={value.enabled}
      {disabled}
      onchange={(e) => commit({ enabled: e.currentTarget.checked })}
    />
    {t("tools.enabled")}
  </label>
  <input
    type="color"
    data-testid="emission-color"
    aria-label={t("tools.color")}
    value={/^#[0-9a-fA-F]{6}$/.test(value.color) ? value.color : "#ffffff"}
    {disabled}
    onchange={(e) => commit({ color: e.currentTarget.value })}
  />
  <label>
    {t("tools.intensity")}
    <input
      type="number"
      data-testid="emission-intensity"
      min="0"
      max="1"
      step="0.05"
      value={value.intensity}
      {disabled}
      onchange={(e) => commitNumber("intensity", e.currentTarget.value)}
    />
  </label>
  <label>
    {t("tools.brightRadius")}
    <input
      type="number"
      data-testid="emission-bright"
      min="0"
      step="0.5"
      value={value.brightRadius}
      {disabled}
      onchange={(e) => commitNumber("brightRadius", e.currentTarget.value)}
    />
  </label>
  <label>
    {t("tools.dimRadius")}
    <input
      type="number"
      data-testid="emission-dim"
      min="0"
      step="0.5"
      value={value.dimRadius}
      {disabled}
      onchange={(e) => commitNumber("dimRadius", e.currentTarget.value)}
    />
  </label>
  <select
    data-testid="emission-falloff"
    aria-label={t("tools.falloff")}
    value={value.falloff?.curve ?? "linear"}
    {disabled}
    onchange={(e) => commit({ falloff: { curve: e.currentTarget.value as FalloffCurve } })}
  >
    {#each falloffCurves as f (f)}<option value={f}>{falloffLabel(f)}</option>{/each}
  </select>
  <label>
    <input
      type="checkbox"
      data-testid="emission-darkness-only"
      checked={value.activeBelowEnv != null}
      {disabled}
      onchange={(e) => setDarknessWindow(e.currentTarget.checked)}
    />
    {t("tools.darknessOnly")}
  </label>
  {#if value.activeBelowEnv != null}
    <label>
      {t("tools.darknessThreshold")}
      <input
        type="range"
        data-testid="emission-darkness-threshold"
        min="0"
        max="1"
        step="0.05"
        value={value.activeBelowEnv}
        {disabled}
        onchange={(e) => commitThreshold(e.currentTarget.value)}
      />
    </label>
  {/if}
  <label>
    <input
      type="checkbox"
      data-testid="emission-cone"
      checked={value.angle != null}
      {disabled}
      onchange={(e) => setCone(e.currentTarget.checked)}
    />
    {t("tools.cone")}
  </label>
  {#if value.angle != null}
    <label>
      {t("tools.coneAngle")}
      <input
        type="number"
        data-testid="emission-cone-angle"
        min="1"
        max="360"
        step="5"
        value={value.angle}
        {disabled}
        onchange={(e) => commitAngle(e.currentTarget.value)}
      />
    </label>
    <label>
      {t("tools.coneRotation")}
      <input
        type="number"
        data-testid="emission-cone-rotation"
        step="5"
        value={value.rotation}
        {disabled}
        onchange={(e) => commitRotation(e.currentTarget.value)}
      />
    </label>
    {#if carriedByToken}
      <label>
        <input
          type="checkbox"
          data-testid="emission-follow-token"
          checked={value.followTokenRotation}
          {disabled}
          onchange={(e) => commit({ followTokenRotation: e.currentTarget.checked })}
        />
        {t("tools.followTokenFacing")}
      </label>
    {/if}
  {/if}
</div>

<style lang="scss">
  .light-emission-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .light-emission-editor input,
  .light-emission-editor select {
    min-height: 32px;

    @media (pointer: coarse) {
      min-height: 44px;
    }
  }
</style>
