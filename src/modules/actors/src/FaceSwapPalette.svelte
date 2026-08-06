<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { selectedFaceNamesFor, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: { tokenId: string | null } = $props();

  // Reactive read of the document store (same bridge as Surface): reading
  // `subscribe()` inside the derived registers a dependency so the palette re-renders on swap.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const selectedFaceToken = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  /** The actor's declared faces map, if the selected token's effective visual is `faces` — drives
   * whether the palette shows at all (a plain image/animated token has nothing to swap). Routed
   * through `selectedFaceNamesFor`, which shares `resolveTokenActor`'s projection with
   * `resolveTokenVisual`, so a per-token `overrides.visual` is honored rather than bypassed. */
  const selectedFaceNames = $derived.by((): string[] => {
    subscribe();
    const tok = selectedFaceToken;
    if (!tok) return [];
    return selectedFaceNamesFor(tok, ctx.documents);
  });

  /** Reads the RAW currently-stored face (never a resolved/defaulted value) — this is the
   * required `old` for the `/engine/face` Update below; the server's field-level optimistic-
   * concurrency check rejects any Update whose `old` doesn't match the actual stored value.
   * @param tok The selected TOKEN document (not an actor) to read the raw `engine.face`
   * override from.
   * @returns The token's raw stored face name, or `null` if none is set.
   * @example
   * ```
   * // private helper; not part of the public API
   * currentFace(tok); // tok.engine.face ?? null
   * ```
   */
  function currentFace(tok: WireDocument): string | null {
    return (tok.engine as { face?: string } | undefined)?.face ?? null;
  }

  /**
   * Dispatches a `/engine/face` Update on the selected token to switch its active face, gated by
   * `ctx.canEdit` — the advisory client-side mirror of the server's Update-path capability
   * check; the server remains authoritative and re-checks independently. A no-op if there is no
   * selected token or the gate refuses.
   * @param faceName The face name to switch to — one of `selectedFaceNames`.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from a face button's `onclick` below
   * swapFace("front");
   * ```
   */
  function swapFace(faceName: string): void {
    const tok = selectedFaceToken;
    if (!tok || !ctx.canEdit(tok, "/engine/face")) return;
    const old = currentFace(tok);
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: "/engine/face", old, new: faceName }] }]);
  }
</script>

{#if selectedFaceToken && selectedFaceNames.length > 0}
  <p class="hint">{t("actors.faceSwapHint")}</p>
  <div class="face-palette">
    {#each selectedFaceNames as name (name)}
      <button type="button" class:active={currentFace(selectedFaceToken) === name} onclick={() => swapFace(name)}>{name}</button>
    {/each}
  </div>
{/if}

<style lang="scss">
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
  .face-palette {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .face-palette button {
    min-width: 44px;
    min-height: 44px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .face-palette button.active {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
