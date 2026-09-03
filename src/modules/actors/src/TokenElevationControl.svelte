<script lang="ts">
  // Per-TOKEN elevation control (mounted by ActorsPanel for the selected token). Elevation is
  // token state (`TokenEngine.elevation`), not actor state — unlike the light/vision override
  // controls it applies to EVERY token, linked or raw. Absent/0 both mean grounded; the write
  // normalizes 0 to `null` so the store keeps one canonical grounded representation.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument, TokenEngine } from "@shadowcat/core";

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

  /** The RAW stored `engine.elevation` — the OCC pre-image for the write below (`null` when
   * absent; the raw-`old` convention, same as `TokenRotationControl`'s `currentRotation`). */
  const rawElevation = $derived((token?.engine as TokenEngine | undefined)?.elevation ?? null);

  const editable = $derived.by((): boolean => {
    subscribe();
    const tok = token;
    // Advisory mirror of the server's Update-path capability check (owner-or-gm for an
    // ordinary engine field); the server remains authoritative and re-checks independently.
    return tok !== null && ctx.canEdit(tok, "/engine/elevation");
  });

  /** Dispatch an `/engine/elevation` Update on the selected token. `0` normalizes to `null`
   * (grounded is stored canonically as absent); a no-op when the normalized value already
   * equals the raw stored one.
   * @param next The new elevation, or `null` for ground level.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the elevation input's onchange
   * setElevation(10);
   * ```
   */
  function setElevation(next: number | null): void {
    const tok = token;
    if (!tok || !editable) return;
    const normalized = next === 0 ? null : next;
    if (normalized === rawElevation) return;
    ctx.dispatchIntent([
      { op: "update", doc_id: tok.id, changes: [{ path: "/engine/elevation", old: rawElevation, new: normalized }] },
    ]);
  }

  /** Parse the input's raw string into an elevation: empty means ground (`null`); a finite
   * number commits; anything else is ignored (never dispatch a NaN).
   * @param raw The input's raw string value.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the elevation input's onchange
   * commitElevation("");
   * ```
   */
  function commitElevation(raw: string): void {
    if (raw.trim() === "") {
      setElevation(null);
      return;
    }
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    setElevation(n);
  }
</script>

{#if token && editable}
  <div class="token-elevation">
    <label>
      {t("actors.tokenElevation")}
      <input
        type="number"
        step="1"
        data-testid="token-elevation"
        value={rawElevation ?? 0}
        onchange={(e) => commitElevation(e.currentTarget.value)}
      />
    </label>
  </div>
{/if}

<style lang="scss">
  .token-elevation {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-elevation input {
    min-height: 44px;
  }
</style>
