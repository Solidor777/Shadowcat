<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable token. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as Surface): reading
  // `subscribe()` inside the derived registers a dependency so the control re-renders
  // when the token's stored rotation changes (locally or from a remote write).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const token = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  /** The RAW stored `engine.rotation` (degrees) — this is the required `old` for the
   * `/engine/rotation` Update below; the server's field-level optimistic-concurrency check
   * rejects an Update whose `old` differs from the stored value. Same raw-`old` convention as
   * `/engine/face` (`FaceSwapPalette`) and `/engine/x`+`/engine/y` (the select/move tool).
   * @param tok The selected TOKEN document (not an actor) to read the raw `engine.rotation`
   * field from.
   * @returns The token's raw stored rotation, in degrees.
   * @example
   * ```
   * // private helper; not part of the public API
   * declare const tok: WireDocument;
   * currentRotation(tok); // tok.engine.rotation
   * ```
   */
  function currentRotation(tok: WireDocument): number {
    return (tok.engine as {
      /** The token's raw stored rotation in degrees; always present (`TokenEngine.rotation`
       * has no default-absent state server-side). */
      rotation?: number;
    } | undefined)?.rotation ?? 0;
  }

  const editable = $derived.by((): boolean => {
    subscribe();
    const tok = token;
    return tok !== null && ctx.canEdit(tok, "/engine/rotation");
  });

  /**
   * Dispatches an `/engine/rotation` Update on the selected token, gated by `ctx.canEdit` — the
   * advisory client-side mirror of the server's Update-path capability check (owner-or-gm for an
   * ordinary engine field); the server remains authoritative and re-checks independently. A
   * no-op if there is no selected token or the gate refuses.
   * @param next The new rotation, in degrees.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the rotation `<input>`'s `oninput`
   * setRotation(90);
   * ```
   */
  function setRotation(next: number): void {
    const tok = token;
    if (!tok || !ctx.canEdit(tok, "/engine/rotation")) return;
    ctx.dispatchIntent([
      {
        op: "update",
        doc_id: tok.id,
        changes: [{ path: "/engine/rotation", old: currentRotation(tok), new: next }],
      },
    ]);
  }
</script>

{#if token && editable}
  <div class="token-rotation">
    <label>
      {t("actors.tokenRotation")}
      <input
        type="number"
        step="1"
        value={currentRotation(token)}
        onchange={(e) => {
          const n = e.currentTarget.valueAsNumber;
          if (Number.isFinite(n)) setRotation(n);
        }}
      />
    </label>
  </div>
{/if}

<style lang="scss">
  .token-rotation {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-rotation input {
    min-height: 44px;
  }
</style>
