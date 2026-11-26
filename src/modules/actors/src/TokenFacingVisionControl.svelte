<script lang="ts">
  // Per-TOKEN facing-vision control (mounted by ActorsPanel for the selected token, GM only). A
  // LINKED token takes an inherit/custom override (`overrides.facing_vision` replaces the actor's
  // value wholesale, like `TokenVisionControl`); an ACTORLESS token carries the value on its own
  // body (`TokenEngine.facing_vision`). An instanced token's embedded copy ignores overrides, so
  // it renders nothing here, mirroring `resolveTokenFacingVision`'s precedence. The value takes
  // effect only on a scene whose resolved `facingVision` switch is on.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveTokenFacingVision, buildUpdate, type WireDocument, type TokenEngine, type TokenOverrides, type FacingVision } from "@shadowcat/core";
  import FacingVisionControl from "./FacingVisionControl.svelte";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable linked or actorless token. */
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
  /** Overrides apply to LINKED tokens only. */
  const linked = $derived(engine?.actor_id != null);
  /** An actorless token carries the value on its own body: no link and no embedded actor. */
  const actorless = $derived(!linked && !token?.embedded?.actor?.[0]);
  /** The raw stored `overrides.facing_vision` — the OCC pre-image for a linked write. */
  const rawOverride = $derived(engine?.overrides?.facing_vision ?? null);
  /** The raw stored `TokenEngine.facing_vision` — the OCC pre-image for an actorless write. */
  const rawOwn = $derived(engine?.facing_vision ?? null);
  /** The value a custom override starts from: the effective (inherited) one, so flipping a
   * token to custom keeps the actor's field of view rather than starting from none. */
  const effective = $derived.by((): FacingVision | null => {
    subscribe();
    return token ? resolveTokenFacingVision(token, ctx.documents) : null;
  });
  /** The mode the UI shows for a linked token: an absent override inherits. */
  const mode = $derived(rawOverride === null ? "inherit" : "custom");

  /** GM-only: a player who could clear their own token's arc would defeat the setting, so the
   * control is not offered to them. The server re-checks the write independently. */
  const editable = $derived.by((): boolean => {
    subscribe();
    return token !== null && ctx.role === "gm" && ctx.canEdit(token, linked ? "/engine/overrides" : "/engine/facing_vision");
  });

  /** Dispatch a whole-object `/engine/overrides` write with only `facing_vision` changed (the
   * whole object is written so the write never depends on nested-pointer intermediate creation;
   * `old` is the RAW stored overrides object).
   * @param next The new override value, or `null` to inherit the actor's again.
   * @example
   * ```
   * // private helper; wired to the mode select and the editor's onCommit
   * setOverride(null); // back to inherit
   * ```
   */
  function setOverride(next: FacingVision | null): void {
    const tok = token;
    if (!tok || !editable) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null };
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/overrides", old: cur, value: { ...base, facing_vision: next ?? undefined } }])]);
  }

  /** Dispatch an `/engine/facing_vision` write on an actorless token.
   * @param next The new value, or `null` to clear it.
   * @example
   * ```
   * // private helper; wired to the editor's onCommit for an actorless token
   * setOwn({ arc: 90, near: 0 });
   * ```
   */
  function setOwn(next: FacingVision | null): void {
    const tok = token;
    if (!tok || !editable) return;
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/facing_vision", old: rawOwn, value: next }])]);
  }

  /** The mode select's commit: inherit clears the override; custom seeds from the stored
   * override, else the effective (inherited) value, else the control's own default.
   * @param next The chosen mode id.
   * @example
   * ```
   * // private helper; wired to the mode select's onchange
   * chooseMode("custom");
   * ```
   */
  function chooseMode(next: string): void {
    if (next === "inherit") setOverride(null);
    else setOverride(rawOverride ?? effective ?? { arc: 120, near: 1 });
  }
</script>

{#if token && editable && (linked || actorless)}
  <div class="token-facing-vision">
    <span>{t("actors.tokenFacingVision")}</span>
    {#if linked}
      <label>
        {t("actors.tokenFacingVision")}
        <select
          data-testid="token-facing-mode"
          value={mode}
          onchange={(e) => chooseMode(e.currentTarget.value)}
        >
          <option value="inherit">{t("actors.facingInherit")}</option>
          <option value="custom">{t("actors.facingCustom")}</option>
        </select>
      </label>
      {#if mode === "custom"}
        <FacingVisionControl value={rawOverride} toggleable={false} onCommit={setOverride} />
      {/if}
    {:else}
      <FacingVisionControl value={rawOwn} onCommit={setOwn} />
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-facing-vision {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-facing-vision select {
    min-height: 44px;
  }
</style>
