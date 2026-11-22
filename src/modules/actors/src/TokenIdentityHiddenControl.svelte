<script lang="ts">
  // Per-TOKEN secret-identity toggle (mounted by ActorsPanel for the selected token). The flag is
  // token state (`TokenEngine.identity_hidden`): while set, a recipient who is neither the token's
  // owner nor a GM receives the token without its name, visual and actor link, so its portrait,
  // faction border and disposition cannot be derived. GM-only to write, mirroring the server's
  // `identity_hidden_touched` gate — it changes what every OTHER viewer perceives.
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

  /** The stored `engine.identity_hidden` — the OCC pre-image for the write below. A stored
   * document that predates the field has no key, so the raw old is `null`; the displayed state
   * is `false` either way. */
  const rawHidden = $derived((token?.engine as Partial<TokenEngine> | undefined)?.identity_hidden ?? null);

  const editable = $derived.by((): boolean => {
    subscribe();
    return token !== null && ctx.role === "gm";
  });

  /** Dispatch an `/engine/identity_hidden` Update on the selected token; a no-op when the value
   * already equals the stored one.
   * @param next The new secret-identity state.
   * @example
   * ```
   * // private helper; invoked from the checkbox's onchange
   * setHidden(true);
   * ```
   */
  function setHidden(next: boolean): void {
    const tok = token;
    if (!tok || !editable) return;
    if (next === (rawHidden ?? false)) return;
    ctx.dispatchIntent([buildUpdate(tok.id, [{ path: "/engine/identity_hidden", old: rawHidden, value: next }])]);
  }
</script>

{#if token && editable}
  <div class="token-identity-hidden">
    <label>
      <input
        type="checkbox"
        data-testid="token-identity-hidden"
        checked={rawHidden === true}
        onchange={(e) => setHidden(e.currentTarget.checked)}
      />
      {t("actors.tokenIdentityHidden")}
    </label>
  </div>
{/if}

<style lang="scss">
  .token-identity-hidden label {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 44px;
  }
</style>
