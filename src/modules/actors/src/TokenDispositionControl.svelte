<script lang="ts">
  // Per-TOKEN disposition override control (mounted by ActorsPanel for the selected token):
  // inherit (the linked faction's stance applies, else neutral) or an explicit friendly /
  // neutral / hostile for this one token. Only a LINKED token takes overrides — an instanced
  // token's embedded copy ignores them and a raw token has no actor to inherit from (mirrors
  // `resolveTokenActor`'s precedence), so both render nothing here.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildUpdate, type FactionStance, type WireDocument, type TokenEngine, type TokenOverrides } from "@shadowcat/core";

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
  /** The raw stored `overrides.disposition` — absent or `null` reads as inherit. */
  const rawOverride = $derived(engine?.overrides?.disposition ?? null);

  const editable = $derived.by((): boolean => {
    subscribe();
    // Advisory mirror of the server's Update-path capability check — `disposition` sits in the
    // owner-writable `overrides` whitelist. The server re-checks authoritatively.
    return token !== null && linked && ctx.canEdit(token, "/engine/overrides");
  });

  /** Dispatch a whole-object `/engine/overrides` write with only `disposition` changed. The
   * whole whitelist object is written (not a nested `/engine/overrides/disposition` pointer) so
   * the write never depends on nested-pointer intermediate creation; `old` is the RAW stored
   * overrides object (the standing raw-`old` OCC convention).
   * @param next The new stance, or `null` to inherit the faction's stance again.
   * @example
   * ```
   * // private helper; wired to the select's onchange
   * setOverride("friendly");
   * ```
   */
  function setOverride(next: FactionStance | null): void {
    const tok = token;
    if (!tok || !editable) return;
    if (next === rawOverride) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null };
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/overrides", old: cur, value: { ...base, disposition: next } }])]);
  }

  /** The select's commit: `"inherit"` clears the override, any other value is a stance.
   * @param value The chosen option value.
   * @example
   * ```
   * // private helper; wired to the select's onchange
   * choose("hostile");
   * ```
   */
  function choose(value: string): void {
    setOverride(value === "inherit" ? null : (value as FactionStance));
  }
</script>

{#if token && linked && editable}
  <div class="token-disposition">
    <label>
      {t("actors.tokenDisposition")}
      <select
        data-testid="token-disposition"
        value={rawOverride ?? "inherit"}
        onchange={(e) => choose(e.currentTarget.value)}
      >
        <option value="inherit">{t("actors.dispositionInherit")}</option>
        <option value="friendly">{t("actors.dispositionFriendly")}</option>
        <option value="neutral">{t("actors.dispositionNeutral")}</option>
        <option value="hostile">{t("actors.dispositionHostile")}</option>
      </select>
    </label>
  </div>
{/if}

<style lang="scss">
  .token-disposition select {
    min-height: 44px;
  }
</style>
