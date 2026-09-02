<script lang="ts">
  // Per-TOKEN movement-tag override control (mounted by ActorsPanel for the selected token):
  // inherit (no override — the resolved actor ∪ faction set applies) / custom (a wholesale
  // replacement tag list). Only a LINKED token takes overrides — an instanced token's
  // embedded copy ignores them and a raw token has no actor to inherit from (mirrors
  // `resolveTokenActor`'s precedence), so both render nothing here. An EMPTY custom list
  // already reads as "this token has no movement tags" (wholesale replacement with no
  // entries). The tags are advisory client-side; authoritative pricing runs server-side.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, MovementTagsEditor } from "@shadowcat/ui-kit";
  import { resolveTokenActor, type WireDocument, type TokenEngine, type TokenOverrides } from "@shadowcat/core";

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
  /** The raw stored `overrides.movement` — the OCC pre-image for every write below. */
  const rawOverride = $derived(engine?.overrides?.movement ?? null);
  /** The effective tags after inheritance — shown read-only in inherit mode, and the base a
   * custom override starts from, so flipping a token to "custom" keeps the resolved set
   * rather than starting from none. */
  const effective = $derived.by((): string[] => {
    subscribe();
    const tok = token;
    return tok ? (resolveTokenActor(tok, ctx.documents)?.movement ?? []) : [];
  });
  /** The override mode the UI shows: absent override = inherit; a stored list (including an
   * empty one — "no movement tags") = custom. */
  const mode = $derived(rawOverride === null ? "inherit" : "custom");

  const editable = $derived.by((): boolean => {
    subscribe();
    // Advisory mirror of the server's Update-path capability check — movement overrides sit in
    // the owner-writable `overrides` whitelist (unlike `overrides.light`, which the server's
    // carried-light gate reserves for GMs). The server re-checks authoritatively.
    return token !== null && linked && ctx.canEdit(token, "/engine/overrides");
  });

  /** Dispatch a whole-object `/engine/overrides` write with only `movement` changed. The whole
   * whitelist object is written (not a nested `/engine/overrides/movement` pointer) so the
   * write never depends on nested-pointer intermediate creation; `old` is the RAW stored
   * overrides object (the standing raw-`old` OCC convention).
   * @param next The new override tag list, or `null` to inherit the resolved set again.
   * @example
   * ```
   * // private helper; wired to the mode select + the editor's onCommit
   * setOverride(null); // back to inherit
   * ```
   */
  function setOverride(next: string[] | null): void {
    const tok = token;
    if (!tok || !editable) return;
    const cur = engine?.overrides ?? null;
    const base: TokenOverrides = cur ?? { name: null, visual: null, size: null, shape: null, vision: null, light: null, movement: null };
    ctx.dispatchIntent([
      { op: "update", doc_id: tok.id, changes: [{ path: "/engine/overrides", old: cur, new: { ...base, movement: next } }] },
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
  <div class="token-movement">
    <label>
      {t("actors.tokenMovement")}
      <select
        data-testid="token-movement-mode"
        value={mode}
        onchange={(e) => chooseMode(e.currentTarget.value)}
      >
        <option value="inherit">{t("actors.movementInherit")}</option>
        <option value="custom">{t("actors.movementCustom")}</option>
      </select>
    </label>
    {#if mode === "custom" && rawOverride}
      <MovementTagsEditor value={rawOverride} onCommit={(next) => setOverride(next)} />
    {:else if effective.length > 0}
      <div class="inherited" data-testid="movement-inherited">
        {#each effective as tag (tag)}<span class="chip">{tag}</span>{/each}
      </div>
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-movement {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-movement select {
    min-height: 44px;
  }
  .inherited {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .inherited .chip {
    padding: 0 var(--space-1);
    min-height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-secondary);
  }
</style>
