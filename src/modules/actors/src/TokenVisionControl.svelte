<script lang="ts">
  // Per-TOKEN vision override control (mounted by ActorsPanel for the selected token):
  // inherit (no override — the linked actor's `engine.vision` applies) / custom (a wholesale
  // replacement assignment list). Only a LINKED token takes overrides — an instanced token's
  // embedded copy ignores them and a raw token has no actor to inherit from (mirrors
  // `resolveTokenActor`'s precedence), so both render nothing here. Unlike the light override
  // there is no suppress mode: an EMPTY custom list already reads as "this token has no
  // senses" (wholesale replacement with no entries).
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, VisionAssignmentsEditor } from "@shadowcat/ui-kit";
  import { resolveTokenActor, resolveVisionModes, type WireDocument, type TokenEngine, type TokenOverrides, type VisionAssignment, type VisionMode } from "@shadowcat/core";

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
  /** The raw stored `overrides.vision` — the OCC pre-image for every write below. */
  const rawOverride = $derived(engine?.overrides?.vision ?? null);
  /** The effective assignments after inheritance — the base a custom override starts from, so
   * flipping a token to "custom" keeps the actor's senses rather than starting from none. */
  const effective = $derived.by((): VisionAssignment[] => {
    subscribe();
    const tok = token;
    return tok ? (resolveTokenActor(tok, ctx.documents)?.visionModes ?? []) : [];
  });
  /** The resolved mode registry the assignment editor's mode select offers. */
  const modes = $derived.by((): VisionMode[] => {
    subscribe();
    return Object.values(resolveVisionModes(ctx.documents));
  });
  /** The override mode the UI shows: absent override = inherit; a stored list (including an
   * empty one — "no senses") = custom. */
  const mode = $derived(rawOverride === null ? "inherit" : "custom");

  const editable = $derived.by((): boolean => {
    subscribe();
    // Advisory mirror of the server's Update-path capability check — vision overrides sit in
    // the owner-writable `overrides` whitelist (unlike `overrides.light`, which the server's
    // carried-light gate reserves for GMs). The server re-checks authoritatively.
    return token !== null && linked && ctx.canEdit(token, "/engine/overrides");
  });

  /** Dispatch a whole-object `/engine/overrides` write with only `vision` changed. The whole
   * whitelist object is written (not a nested `/engine/overrides/vision` pointer) so the write
   * never depends on nested-pointer intermediate creation; `old` is the RAW stored overrides
   * object (the standing raw-`old` OCC convention).
   * @param next The new override assignment list, or `null` to inherit the actor's senses again.
   * @example
   * ```
   * // private helper; wired to the mode select + the editor's onCommit
   * setOverride(null); // back to inherit
   * ```
   */
  function setOverride(next: VisionAssignment[] | null): void {
    const tok = token;
    if (!tok || !editable) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null , aura: null, sound: null, vfx: null };
    ctx.dispatchIntent([
      { op: "update", doc_id: tok.id, changes: [{ path: "/engine/overrides", old: cur, new: { ...base, vision: next } }] },
    ]);
  }

  /** The mode select's commit: inherit clears the override; custom seeds from the stored
   * override, else the effective (inherited) list.
   * @param next The chosen mode id.
   * @example
   * ```
   * // private helper; wired to the mode select's onchange
   * chooseMode("custom");
   * ```
   */
  function chooseMode(next: string): void {
    if (next === "inherit") setOverride(null);
    else setOverride([...(rawOverride ?? effective)]);
  }
</script>

{#if token && linked && editable}
  <div class="token-vision">
    <label>
      {t("actors.tokenVision")}
      <select
        data-testid="token-vision-mode"
        value={mode}
        onchange={(e) => chooseMode(e.currentTarget.value)}
      >
        <option value="inherit">{t("actors.visionInherit")}</option>
        <option value="custom">{t("actors.visionCustom")}</option>
      </select>
    </label>
    {#if mode === "custom" && rawOverride}
      <VisionAssignmentsEditor value={rawOverride} {modes} onCommit={(next) => setOverride(next)} />
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-vision {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-vision select {
    min-height: 44px;
  }
</style>
