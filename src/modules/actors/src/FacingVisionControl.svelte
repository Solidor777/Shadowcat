<script lang="ts">
  // Value-only editor for a creature's facing-limited field of view (`FacingVision`): an enable
  // checkbox, the forward arc's width as a slider plus a number field, and the all-round
  // awareness radius in cells. The OWNING surface (an actor row, a token-override control) owns
  // the dispatch, so the raw-`old` OCC pre-image stays at the call site that knows the path.
  // Every field commits on `change` (slider release / field blur), never per `input` tick, so one
  // gesture is one write. The bounds match the server's `FacingVision::validate`: the arc is in
  // (0, 360] degrees and the awareness radius is >= 0.
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { FacingVision } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  /** The value a freshly enabled creature starts from: a wide forward arc with a one-cell
   * all-round bubble, so enabling the control never blinds a token to its own neighbours. */
  const ENABLED_DEFAULT: FacingVision = { arc: 120, near: 1 };

  let { value, disabled = false, toggleable = true, onCommit }: {
    /** The value being edited; `null` renders the control disabled-off (all-round sight). */
    value: FacingVision | null;
    /** Read-only mode (a surface the viewer may not edit). */
    disabled?: boolean;
    /** Whether the enable checkbox is offered. A surface whose absent value means something
     * other than "all-round" (a token override, where absent inherits) hides it and passes a
     * non-null `value`; an arc of 360 is then the way to remove the restriction. */
    toggleable?: boolean;
    /** Commit callback: receives the WHOLE new value, or `null` when the checkbox is cleared. */
    onCommit: (next: FacingVision | null) => void;
  } = $props();

  /** Flip the enable checkbox: on commits `ENABLED_DEFAULT`, off commits `null`.
   * @param on Whether the checkbox is now checked.
   * @example
   * ```
   * // private helper; wired to the checkbox's onchange
   * setEnabled(true);
   * ```
   */
  function setEnabled(on: boolean): void {
    onCommit(on ? { ...ENABLED_DEFAULT } : null);
  }

  /** Commit a new arc width. A non-finite or non-positive entry is ignored; a width above 360
   * is clamped to the full circle (no restriction).
   * @param raw The entered width in degrees.
   * @example
   * ```
   * // private helper; wired to the slider's and number field's onchange
   * setArc(90);
   * ```
   */
  function setArc(raw: number): void {
    if (value === null || !Number.isFinite(raw) || raw <= 0) return;
    onCommit({ ...value, arc: Math.min(raw, 360) });
  }

  /** Commit a new all-round awareness radius. A non-finite or negative entry is ignored.
   * @param raw The entered radius in grid cells.
   * @example
   * ```
   * // private helper; wired to the awareness field's onchange
   * setNear(2);
   * ```
   */
  function setNear(raw: number): void {
    if (value === null || !Number.isFinite(raw) || raw < 0) return;
    onCommit({ ...value, near: raw });
  }
</script>

<div class="facing-vision">
  {#if toggleable}
    <label>
      <input
        type="checkbox"
        data-testid="facing-enabled"
        checked={value !== null}
        {disabled}
        onchange={(e) => setEnabled(e.currentTarget.checked)}
      />
      {t("actors.facingVisionEnabled")}
    </label>
  {/if}
  {#if value !== null}
    <label>
      {t("actors.facingArc")}
      <input
        type="range"
        min="1"
        max="360"
        step="1"
        data-testid="facing-arc-slider"
        aria-label={t("actors.facingArcSlider")}
        value={value.arc}
        {disabled}
        onchange={(e) => setArc(e.currentTarget.valueAsNumber)}
      />
      <input
        type="number"
        min="1"
        max="360"
        step="1"
        data-testid="facing-arc"
        value={value.arc}
        {disabled}
        onchange={(e) => setArc(e.currentTarget.valueAsNumber)}
      />
    </label>
    <label>
      {t("actors.facingNear")}
      <input
        type="number"
        min="0"
        step="0.5"
        data-testid="facing-near"
        value={value.near}
        {disabled}
        onchange={(e) => setNear(e.currentTarget.valueAsNumber)}
      />
    </label>
  {/if}
</div>

<style lang="scss">
  .facing-vision {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .facing-vision label {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .facing-vision input[type="number"],
  .facing-vision input[type="range"] {
    min-height: 32px;

    @media (pointer: coarse) {
      min-height: 44px;
    }
  }
  .facing-vision input[type="checkbox"] {
    min-width: 24px;
    min-height: 24px;
  }
</style>
