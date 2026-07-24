<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { effectiveOwner, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: { tokenId: string | null } = $props();

  // Reactive read of the document store (same bridge as Surface.svelte): reading
  // `subscribe()` inside the derived registers a dependency so the control re-renders
  // when the token — or the ACTOR it inherits from — changes owner.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const token = $derived.by((): WireDocument | null => {
    subscribe();
    if (!tokenId) return null;
    return ctx.documents.get(tokenId) ?? null;
  });

  /** The RAW stored per-token override (never the resolved effective owner) — this is
   * the required `old` for the `/owner` Update below; the server's field-level
   * optimistic-concurrency check rejects an Update whose `old` differs from the stored
   * value. Same raw-`old` convention as the `/engine/face` swap. */
  const overrideOwner = $derived(token?.owner ?? null);

  /** The rule's answer for this token: the override, else the LINKED actor's owner.
   * Resolved through the same `effectiveOwner` the advisory `canEdit` gate uses, so the
   * label can never disagree with what the UI actually permits. */
  const resolved = $derived.by((): string | null => {
    subscribe();
    return token ? effectiveOwner(token, ctx.documents) : null;
  });

  const memberEntries = $derived([...ctx.members.entries()]);

  function label(userId: string | null): string {
    if (!userId) return t("actors.ownerNobody");
    return ctx.members.get(userId) ?? userId;
  }

  function setOverride(next: string | null): void {
    const tok = token;
    if (!tok) return;
    ctx.dispatchIntent([
      { op: "update", doc_id: tok.id, changes: [{ path: "/owner", old: overrideOwner, new: next }] },
    ]);
  }
</script>

{#if token && ctx.role === "gm"}
  <div class="token-owner">
    <label>
      {t("actors.tokenOwner")}
      <select
        value={overrideOwner ?? ""}
        onchange={(e) => setOverride(e.currentTarget.value || null)}
      >
        <option value="">{t("actors.ownerInherit")}</option>
        {#each memberEntries as [id, username] (id)}
          <option value={id}>{username}</option>
        {/each}
      </select>
    </label>
    <p class="hint">{t("actors.tokenOwnerEffective", { owner: label(resolved) })}</p>
  </div>
{/if}

<style lang="scss">
  .token-owner {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .token-owner select {
    min-height: 44px;
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
</style>
