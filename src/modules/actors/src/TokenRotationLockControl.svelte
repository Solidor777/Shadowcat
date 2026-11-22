<script lang="ts">
  // Per-TOKEN rotation-lock control (mounted by ActorsPanel for the selected token). The lock is
  // token state (`TokenEngine.lock_rotation`), not actor state — like elevation it applies to
  // EVERY token, linked or raw, with no `overrides` indirection. A locked token is never
  // auto-rotated by a move, whatever the scene's `autoRotateTokens` resolves to.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildUpdate, type WireDocument, type TokenEngine } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable token. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as the sibling per-token controls).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const token = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  /** The stored `engine.lock_rotation` — the OCC pre-image for the write below. A stored
   * document that predates the field has no key, so the raw old is `null`; the displayed state
   * is `false` either way. */
  const rawLock = $derived((token?.engine as Partial<TokenEngine> | undefined)?.lock_rotation ?? null);

  const editable = $derived.by((): boolean => {
    subscribe();
    const tok = token;
    // Advisory mirror of the server's Update-path capability check (owner-or-gm for an
    // ordinary engine field); the server remains authoritative and re-checks independently.
    return tok !== null && ctx.canEdit(tok, "/engine/lock_rotation");
  });

  /** Dispatch an `/engine/lock_rotation` Update on the selected token; a no-op when the value
   * already equals the stored one.
   * @param next The new lock state.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the checkbox's onchange
   * setLock(true);
   * ```
   */
  function setLock(next: boolean): void {
    const tok = token;
    if (!tok || !editable) return;
    if (next === (rawLock ?? false)) return;
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/lock_rotation", old: rawLock, value: next }])]);
  }
</script>

{#if token && editable}
  <div class="token-rotation-lock">
    <label>
      <input
        type="checkbox"
        data-testid="token-rotation-lock"
        checked={rawLock === true}
        onchange={(e) => setLock(e.currentTarget.checked)}
      />
      {t("actors.tokenRotationLock")}
    </label>
  </div>
{/if}

<style lang="scss">
  .token-rotation-lock label {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 44px;
  }
</style>
