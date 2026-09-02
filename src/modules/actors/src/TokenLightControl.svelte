<script lang="ts">
  // Per-TOKEN carried-light override control (mounted by ActorsPanel for the selected token):
  // inherit (no override) / suppress (an enabled:false override — the documented suppress path)
  // / custom (a replacement emission). Only a LINKED token takes overrides — an instanced
  // token's embedded copy ignores them and a raw token has no actor to inherit from (mirrors
  // `resolveTokenActor`'s precedence), so both render nothing here.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, LightEmissionEditor } from "@shadowcat/ui-kit";
  import { resolveTokenActor, DEFAULT_LIGHT_EMISSION, type WireDocument, type TokenEngine, type TokenOverrides, type LightEmission } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable LINKED token. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as the sibling per-token controls).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const token = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  /** The token's raw engine body. */
  const engine = $derived(token?.engine as TokenEngine | undefined);
  /** Overrides apply to LINKED tokens only (`resolveTokenActor`'s precedence). */
  const linked = $derived(engine?.actor_id != null);
  /** The raw stored `overrides.light` — the OCC pre-image for every write below. */
  const rawOverride = $derived(engine?.overrides?.light ?? null);
  /** The effective emission after inheritance — the base a suppress/custom override starts
   * from, so flipping a token to "suppress" keeps the actor's radii/color rather than
   * inventing values. */
  const effective = $derived.by((): LightEmission | null => {
    subscribe();
    const tok = token;
    return tok ? (resolveTokenActor(tok, ctx.documents)?.light ?? null) : null;
  });
  /** The override mode the UI shows: absent override = inherit; a disabled override =
   * suppress; an enabled one = custom. */
  const mode = $derived(rawOverride === null ? "inherit" : rawOverride.enabled ? "custom" : "suppress");

  const editable = $derived.by((): boolean => {
    subscribe();
    // GM-only, mirroring the server's carried-light write gate (`carried_light_touched`):
    // an emission joins the shared illumination field, so it is not owner-writable like the
    // sibling override fields. Advisory; the server re-checks authoritatively.
    return token !== null && linked && ctx.role === "gm";
  });

  /** Dispatch a whole-object `/engine/overrides` write with only `light` changed. The whole
   * whitelist object is written (not a nested `/engine/overrides/light` pointer) so the write
   * never depends on nested-pointer intermediate creation; `old` is the RAW stored overrides
   * object (the standing raw-`old` OCC convention).
   * @param next The new override emission, or `null` to inherit the actor's light again.
   * @example
   * ```
   * // private helper; wired to the mode select + the editor's onCommit
   * setOverride(null); // back to inherit
   * ```
   */
  function setOverride(next: LightEmission | null): void {
    const tok = token;
    if (!tok || !editable) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null };
    ctx.dispatchIntent([
      { op: "update", doc_id: tok.id, changes: [{ path: "/engine/overrides", old: cur, new: { ...base, light: next } }] },
    ]);
  }

  /** The mode select's commit: inherit clears the override; suppress writes the effective (or
   * default) emission disabled; custom writes it enabled.
   * @param next The chosen mode id.
   * @example
   * ```
   * // private helper; wired to the mode select's onchange
   * chooseMode("suppress");
   * ```
   */
  function chooseMode(next: string): void {
    if (next === "inherit") setOverride(null);
    else if (next === "suppress") setOverride({ ...(effective ?? { ...DEFAULT_LIGHT_EMISSION }), enabled: false });
    else setOverride({ ...(rawOverride ?? effective ?? { ...DEFAULT_LIGHT_EMISSION }), enabled: true });
  }
</script>

{#if token && linked && editable}
  <div class="token-light">
    <label>
      {t("actors.tokenLight")}
      <select
        data-testid="token-light-mode"
        value={mode}
        onchange={(e) => chooseMode(e.currentTarget.value)}
      >
        <option value="inherit">{t("actors.lightInherit")}</option>
        <option value="suppress">{t("actors.lightSuppress")}</option>
        <option value="custom">{t("actors.lightCustom")}</option>
      </select>
    </label>
    {#if mode === "custom" && rawOverride}
      <LightEmissionEditor value={rawOverride} onCommit={(next) => setOverride(next)} />
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-light {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-light select {
    min-height: 44px;
  }
</style>
