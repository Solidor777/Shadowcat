<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { listAssets, type AuraEmission, type SoundEmission, type VfxEmission, type VfxAnchor } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let {
    aura,
    sound,
    vfx,
    onAura,
    onSound,
    onVfx,
  }: {
    /** The current aura emission, or `null` for none (the section renders collapsed). */
    aura: AuraEmission | null;
    /** The current sound emission, or `null` for none. */
    sound: SoundEmission | null;
    /** The current VFX emission, or `null` for none. */
    vfx: VfxEmission | null;
    /** Called with the replacement aura payload (or `null` when the section toggles off) on
     * every edit. */
    onAura: (v: AuraEmission | null) => void;
    /** Called with the replacement sound payload (or `null` when the section toggles off). */
    onSound: (v: SoundEmission | null) => void;
    /** Called with the replacement VFX payload (or `null` when the section toggles off). */
    onVfx: (v: VfxEmission | null) => void;
  } = $props();

  let audioAssets = $state<Asset[]>([]);
  let visualAssets = $state<Asset[]>([]);

  /**
   * Refetches the world's assets into the two picker lists — `audio/*` for the sound section,
   * `image/*` + `video/*` for the VFX section. Same load/reconcile pattern as
   * `VisualKindEditor`'s `refreshAssets`.
   * @returns Nothing; assigns the component's own `$state` lists.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the `$effect` below
   * refreshAssets();
   * ```
   */
  function refreshAssets(): void {
    void listAssets(ctx.world).then((a) => {
      audioAssets = a.filter((x) => x.content_type.startsWith("audio/"));
      visualAssets = a.filter((x) => x.content_type.startsWith("image/") || x.content_type.startsWith("video/"));
      // Every record carries the true, current version — reconciling on each load self-heals a
      // uuid whose cache-bust state went stale from a missed AssetChanged frame.
      ctx.assets.reconcile(a);
    });
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  /**
   * The payload a section toggles ON to — every field present with a sane default, so a toggled-on
   * emission is always a complete, ingress-valid shape (`asset` stays empty until picked; the
   * server rejects an empty asset id, which the caller surfaces as a rejected intent).
   * @returns A fresh default payload per call (no aliasing between edits).
   * @example
   * ```
   * // private helpers; not part of the public API
   * defaultAura(); // { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true }
   * ```
   */
  function defaultAura(): AuraEmission {
    return { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true };
  }
  /** See `defaultAura` — the sound section's toggle-on payload.
   * @returns A fresh default sound payload per call.
   * @example
   * ```
   * // private helper; not part of the public API
   * defaultSound(); // { asset: "", radius: 5, volume: 0.8, loop: true, enabled: true }
   * ```
   */
  function defaultSound(): SoundEmission {
    return { asset: "", radius: 5, volume: 0.8, loop: true, enabled: true };
  }
  /** See `defaultAura` — the VFX section's toggle-on payload.
   * @returns A fresh default VFX payload per call.
   * @example
   * ```
   * // private helper; not part of the public API
   * defaultVfx(); // { asset: "", anchor: "token", loop: true, enabled: true }
   * ```
   */
  function defaultVfx(): VfxEmission {
    return { asset: "", anchor: "token", loop: true, enabled: true };
  }
</script>

<div class="emission-editor">
  <label class="emission-toggle">
    <input
      type="checkbox"
      aria-label={t("actors.aura")}
      checked={aura !== null}
      onchange={(e) => onAura(e.currentTarget.checked ? defaultAura() : null)}
    />
    {t("actors.aura")}
  </label>
  {#if aura}
    <div class="emission-fields">
      <label>{t("actors.auraColor")}
        <input type="color" aria-label={t("actors.auraColor")} value={aura.color} onchange={(e) => onAura({ ...aura, color: e.currentTarget.value })} oninput={(e) => onAura({ ...aura, color: e.currentTarget.value })} />
      </label>
      <!-- value + onchange/oninput (not bind:value): bind:value on a number input reacts only to
           input events; explicit handlers keep this in sync with fireEvent.change in tests too. -->
      <label>{t("actors.emissionOpacity")}
        <input type="number" min="0" max="1" step="0.05" aria-label={t("actors.emissionOpacity")} value={aura.opacity} onchange={(e) => onAura({ ...aura, opacity: Number(e.currentTarget.value) })} oninput={(e) => onAura({ ...aura, opacity: Number(e.currentTarget.value) })} />
      </label>
      <label>{t("actors.auraRadius")}
        <input type="number" min="0" step="0.5" aria-label={t("actors.auraRadius")} value={aura.radius} onchange={(e) => onAura({ ...aura, radius: Number(e.currentTarget.value) })} oninput={(e) => onAura({ ...aura, radius: Number(e.currentTarget.value) })} />
      </label>
      <label><input type="checkbox" aria-label={t("actors.emissionEnabled")} checked={aura.enabled} onchange={(e) => onAura({ ...aura, enabled: e.currentTarget.checked })} /> {t("actors.emissionEnabled")}</label>
    </div>
  {/if}

  <label class="emission-toggle">
    <input
      type="checkbox"
      aria-label={t("actors.sound")}
      checked={sound !== null}
      onchange={(e) => onSound(e.currentTarget.checked ? defaultSound() : null)}
    />
    {t("actors.sound")}
  </label>
  {#if sound}
    <div class="emission-fields">
      <label>{t("actors.emissionAsset")}
        <select aria-label={t("actors.emissionAsset")} value={sound.asset} onchange={(e) => onSound({ ...sound, asset: e.currentTarget.value })}>
          <option value="">—</option>
          {#each audioAssets as a (a.id)}<option value={a.id}>{a.original_name}</option>{/each}
        </select>
      </label>
      <label>{t("actors.emissionRadius")}
        <input type="number" min="0" step="0.5" aria-label={t("actors.soundRadius")} value={sound.radius} onchange={(e) => onSound({ ...sound, radius: Number(e.currentTarget.value) })} oninput={(e) => onSound({ ...sound, radius: Number(e.currentTarget.value) })} />
      </label>
      <label>{t("actors.emissionVolume")}
        <input type="number" min="0" max="1" step="0.05" aria-label={t("actors.emissionVolume")} value={sound.volume} onchange={(e) => onSound({ ...sound, volume: Number(e.currentTarget.value) })} oninput={(e) => onSound({ ...sound, volume: Number(e.currentTarget.value) })} />
      </label>
      <label><input type="checkbox" aria-label={t("actors.animLoop")} checked={sound.loop} onchange={(e) => onSound({ ...sound, loop: e.currentTarget.checked })} /> {t("actors.animLoop")}</label>
      <label><input type="checkbox" aria-label={t("actors.soundEnabled")} checked={sound.enabled} onchange={(e) => onSound({ ...sound, enabled: e.currentTarget.checked })} /> {t("actors.emissionEnabled")}</label>
    </div>
  {/if}

  <label class="emission-toggle">
    <input
      type="checkbox"
      aria-label={t("actors.vfx")}
      checked={vfx !== null}
      onchange={(e) => onVfx(e.currentTarget.checked ? defaultVfx() : null)}
    />
    {t("actors.vfx")}
  </label>
  {#if vfx}
    <div class="emission-fields">
      <label>{t("actors.vfxAsset")}
        <select aria-label={t("actors.vfxAsset")} value={vfx.asset} onchange={(e) => onVfx({ ...vfx, asset: e.currentTarget.value })}>
          <option value="">—</option>
          {#each visualAssets as a (a.id)}<option value={a.id}>{a.original_name}</option>{/each}
        </select>
      </label>
      <label>{t("actors.vfxAnchor")}
        <select aria-label={t("actors.vfxAnchor")} value={vfx.anchor} onchange={(e) => onVfx({ ...vfx, anchor: e.currentTarget.value as VfxAnchor })}>
          <option value="token">{t("actors.vfxAnchorToken")}</option>
          <option value="above">{t("actors.vfxAnchorAbove")}</option>
          <option value="below">{t("actors.vfxAnchorBelow")}</option>
        </select>
      </label>
      <label><input type="checkbox" aria-label={t("actors.vfxLoop")} checked={vfx.loop} onchange={(e) => onVfx({ ...vfx, loop: e.currentTarget.checked })} /> {t("actors.animLoop")}</label>
      <label><input type="checkbox" aria-label={t("actors.vfxEnabled")} checked={vfx.enabled} onchange={(e) => onVfx({ ...vfx, enabled: e.currentTarget.checked })} /> {t("actors.emissionEnabled")}</label>
    </div>
  {/if}
</div>

<style lang="scss">
  .emission-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .emission-toggle {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 32px;
  }
  .emission-fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-left: var(--space-2);
  }
  .emission-fields label {
    min-height: 32px;
  }
</style>
