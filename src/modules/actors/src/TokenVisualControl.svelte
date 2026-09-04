<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveTokenActor, type WireDocument, type TokenEngine, type TokenOverrides, type TokenVisual, type Condition, type ConditionRegistryEngine } from "@shadowcat/core";
  import VisualKindEditor from "./VisualKindEditor.svelte";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable LINKED token. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as Surface): reading
  // `subscribe()` inside the derived registers a dependency so the control re-renders
  // when the token's stored overrides change (locally or from a remote write).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  /** The selected token, or `null` when absent OR not actor-linked: `TokenOverrides` projects
   * only for a linked token (`resolveTokenActor` reads `overrides` only in its linked branch),
   * so an instanced token's override fields would be inert — the control hides instead. Same
   * gating rule as `TokenEmissionControl`. */
  const linkedToken = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    const tok = ctx.documents.get(tokenId) ?? null;
    if (!tok) return null;
    return (tok.engine as TokenEngine | undefined)?.actor_id ? tok : null;
  });

  const editable = $derived.by((): boolean => {
    subscribe();
    const tok = linkedToken;
    return tok !== null && ctx.canEdit(tok, "/engine/overrides");
  });

  const conditionOptions = $derived.by((): [string, Condition][] => {
    subscribe();
    const reg = ctx.documents.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(reg?.conditions ?? {});
  });

  /** The selected token's EFFECTIVE visual — the linked actor's `ActorEngine.visual` with any
   * stored `TokenOverrides.visual` already folded in by `resolveTokenActor`'s projection, the
   * same read `resolveTokenVisual` resolves from — so the editor initializes from what the token
   * actually renders, never from blank. `null` hides the control (a dangling link has no actor
   * visual to edit against). */
  const effectiveVisual = $derived.by((): TokenVisual | null => {
    subscribe();
    const tok = linkedToken;
    if (!tok) return null;
    return resolveTokenActor(tok, ctx.documents)?.visual ?? null;
  });

  /** The editor's last built visual, or `null` while its active kind's data is incomplete —
   * gates the apply button exactly like `ActorsPanel`'s `pendingVisual` gates its create
   * button. */
  let pendingVisual = $state<TokenVisual | null>(null);

  /** The RAW stored `TokenOverrides` — the read site for every OCC `old` below; the server's
   * field-level optimistic-concurrency check rejects an Update whose `old` differs from the
   * stored value. Same raw-`old` convention as `TokenEmissionControl`.
   * @param tok The selected LINKED token document to read the raw `engine.overrides` from.
   * @returns The token's raw stored override whitelist, or `null` if none is set.
   * @example
   * ```
   * // private helper; not part of the public API
   * declare const tok: WireDocument;
   * rawOverrides(tok); // tok.engine.overrides ?? null
   * ```
   */
  function rawOverrides(tok: WireDocument): TokenOverrides | null {
    return (tok.engine as TokenEngine | undefined)?.overrides ?? null;
  }

  /**
   * Dispatches the editor's built visual as a `/engine/overrides/visual` Update on the selected
   * token — a wholesale per-token visual override, replacing whatever the token inherits from
   * its actor — gated by `ctx.canEdit`, the advisory client-side mirror of the server's
   * Update-path capability check; the server remains authoritative and re-checks independently.
   * A no-op if there is no linked selected token, the gate refuses, or the editor's build is
   * incomplete (the apply button is disabled on `null`).
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the apply button below
   * applyVisual();
   * ```
   */
  function applyVisual(): void {
    const tok = linkedToken;
    if (!tok || !ctx.canEdit(tok, "/engine/overrides")) return;
    // `$state.snapshot` strips the deep-reactive Proxy wrapping `$state` applies to the editor's
    // built literal — an unwrapped reactive value embedded in the document would later fail
    // `structuredClone`. Same read-site convention as `ActorsPanel.create()`'s `pendingVisual`.
    const visual = $state.snapshot(pendingVisual);
    if (!visual) return;
    const old = rawOverrides(tok)?.visual ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/engine/overrides/visual", old, new: visual }] }]);
  }

  /**
   * Clears the token's stored visual override — a `/engine/overrides/visual` Update writing
   * `null`, so the token falls back to inheriting its actor's visual — under the same
   * `ctx.canEdit` gate and raw-`old` OCC convention as `applyVisual`. Rendered only while a
   * stored override exists, so `old` is never `null` here in practice.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the clear-override button below
   * clearVisual();
   * ```
   */
  function clearVisual(): void {
    const tok = linkedToken;
    if (!tok || !ctx.canEdit(tok, "/engine/overrides")) return;
    const old = rawOverrides(tok)?.visual ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/engine/overrides/visual", old, new: null }] }]);
  }
</script>

{#if linkedToken && editable && effectiveVisual}
  <div class="token-visual">
    <p class="hint">{t("actors.tokenVisualHint")}</p>
    <!-- Remount per selected token: `VisualKindEditor`'s `initial` prop is read once at mount,
         so a selection change must construct a fresh editor to initialize from the new token's
         effective visual. -->
    {#key linkedToken.id}
      <VisualKindEditor conditionOptions={conditionOptions} initial={effectiveVisual} onBuild={(v) => (pendingVisual = v)} />
    {/key}
    <button type="button" disabled={!pendingVisual} onclick={applyVisual}>{t("actors.applyVisual")}</button>
    {#if rawOverrides(linkedToken)?.visual}
      <button type="button" onclick={clearVisual}>{t("actors.clearVisualOverride")}</button>
    {/if}
  </div>
{/if}

<style lang="scss">
  .token-visual {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
</style>
