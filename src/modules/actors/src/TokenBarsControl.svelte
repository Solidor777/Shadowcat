<script lang="ts">
  // Per-TOKEN nameplate and bars override control (mounted by ActorsPanel for the selected
  // token): the nameplate mode and the bar list each inherit the linked actor's value or carry
  // this token's own. Only a LINKED token takes overrides — an instanced token's embedded copy
  // ignores them and a raw token has no actor to inherit from (mirrors `resolveTokenActor`'s
  // precedence), so both render nothing here.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildUpdate, type ActorEngine, type BarAttribute, type BarDisplayMode, type TokenEngine, type TokenOverrides, type WireDocument } from "@shadowcat/core";
  import BarModeSelect from "./BarModeSelect.svelte";
  import BarsEditor from "./BarsEditor.svelte";

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
  /** Overrides apply to LINKED tokens only. */
  const linked = $derived(engine?.actor_id != null);
  /** The raw stored `overrides.bars` — absent reads as inherit. */
  const rawBars = $derived(engine?.overrides?.bars ?? null);
  /** The raw stored `overrides.nameplate_mode` — absent reads as inherit. */
  const rawNameplateMode = $derived(engine?.overrides?.nameplate_mode ?? null);
  /** The linked actor's own bars, the seed for switching to a custom list. */
  const actorBars = $derived.by((): BarAttribute[] => {
    subscribe();
    const id = engine?.actor_id;
    if (!id) return [];
    return (ctx.documents.get(id)?.engine as ActorEngine | undefined)?.bars ?? [];
  });

  const editable = $derived.by((): boolean => {
    subscribe();
    // Advisory mirror of the server's Update-path capability check; the server re-checks.
    return token !== null && linked && ctx.canEdit(token, "/engine/overrides");
  });

  /** Dispatch a whole-object `/engine/overrides` write with `patch` applied. A patched key set to
   * `undefined` is dropped from the object, the canonical "inherit" (the optional fields are
   * absent, never `null`). The whole whitelist object is written (never a nested pointer) so the
   * write does not depend on nested-pointer intermediate creation; `old` is the RAW stored
   * overrides object (the standing raw-`old` OCC convention).
   * @param patch The override fields to change.
   * @example
   * ```
   * // private helper; wired to the controls' handlers
   * writeOverrides({ nameplate_mode: "always" });
   * ```
   */
  function writeOverrides(patch: Partial<Pick<TokenOverrides, "bars" | "nameplate_mode">>): void {
    const tok = token;
    if (!tok || !editable) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null, aura: null, sound: null, vfx: null, disposition: null };
    const next: TokenOverrides = { ...base, ...patch };
    for (const key of ["bars", "nameplate_mode"] as const) if (next[key] === undefined) delete next[key];
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/overrides", old: cur, value: $state.snapshot(next) }])]);
  }

  /** Switches the bar list between inherit and this token's own copy of the actor's list.
   * @param custom `true` to start a custom list seeded from the actor's, `false` to inherit.
   * @example
   * ```
   * // private helper; wired to the bars select
   * setCustom(true);
   * ```
   */
  function setCustom(custom: boolean): void {
    if (custom === (rawBars !== null)) return;
    writeOverrides({ bars: custom ? structuredClone($state.snapshot(actorBars)) : undefined });
  }
</script>

{#if token && linked && editable}
  <div class="token-bars">
    <label>
      {t("actors.tokenNameplateMode")}
      <BarModeSelect
        value={rawNameplateMode}
        label={t("actors.tokenNameplateMode")}
        inheritLabel={t("actors.nameplateInherit")}
        testId="token-nameplate-mode"
        onChange={(mode: BarDisplayMode | null) => writeOverrides({ nameplate_mode: mode ?? undefined })}
      />
    </label>
    <label>
      {t("actors.tokenBars")}
      <select data-testid="token-bars-source" value={rawBars === null ? "inherit" : "custom"} onchange={(e) => setCustom(e.currentTarget.value === "custom")}>
        <option value="inherit">{t("actors.barsInherit")}</option>
        <option value="custom">{t("actors.barsCustom")}</option>
      </select>
    </label>
    {#if rawBars !== null}
      <BarsEditor bars={rawBars} onCommit={(next) => writeOverrides({ bars: next })} />
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-bars {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-bars select {
    min-height: 44px;
  }
</style>
