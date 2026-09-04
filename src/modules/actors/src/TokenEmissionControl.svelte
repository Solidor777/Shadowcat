<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument, TokenEngine, TokenOverrides, AuraEmission, SoundEmission, VfxEmission } from "@shadowcat/core";
  import EmissionEditor from "./EmissionEditor.svelte";

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
   * so an instanced token's override fields would be inert — the control hides instead. */
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

  /** The RAW stored `TokenOverrides` — the read site for every OCC `old` below; the server's
   * field-level optimistic-concurrency check rejects an Update whose `old` differs from the
   * stored value. Same raw-`old` convention as `/engine/face` (`FaceSwapPalette`).
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
   * Dispatches a `/engine/overrides/<kind>` Update on the selected token — one emission kind at
   * a time, wholesale per kind (a kind's payload replaces or clears, never merges) — gated by
   * `ctx.canEdit`, the advisory client-side mirror of the server's Update-path capability check;
   * the server remains authoritative and re-checks independently. A no-op if there is no linked
   * selected token or the gate refuses.
   * @param kind Which override field to write (`aura`/`sound`/`vfx`).
   * @param v The replacement emission payload, or `null` to clear the override.
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the EmissionEditor callbacks below
   * write("aura", null);
   * ```
   */
  function write(kind: "aura" | "sound" | "vfx", v: AuraEmission | SoundEmission | VfxEmission | null): void {
    const tok = linkedToken;
    if (!tok || !ctx.canEdit(tok, "/engine/overrides")) return;
    const old = rawOverrides(tok)?.[kind] ?? null;
    ctx.dispatchIntent([{ op: "update", doc_id: tok.id, changes: [{ path: `/engine/overrides/${kind}`, old, new: v }] }]);
  }
</script>

{#if linkedToken && editable}
  <div class="token-emissions">
    <p class="hint">{t("actors.tokenEmissionsHint")}</p>
    <EmissionEditor
      aura={rawOverrides(linkedToken)?.aura ?? null}
      sound={rawOverrides(linkedToken)?.sound ?? null}
      vfx={rawOverrides(linkedToken)?.vfx ?? null}
      onAura={(v) => write("aura", v)}
      onSound={(v) => write("sound", v)}
      onVfx={(v) => write("vfx", v)}
    />
  </div>
{/if}

<style lang="scss">
  .token-emissions {
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
