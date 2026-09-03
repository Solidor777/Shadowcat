<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { effectiveOwner, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { tokenId }: {
    /** The currently selected token id, or `null` when no single token is selected — the
     * control renders nothing without a resolvable token. */
    tokenId: string | null;
  } = $props();

  // Reactive read of the document store (same bridge as Surface): reading
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

  /**
   * Display label for a user id — the member's username if known, else the raw id (e.g. a
   * since-removed member) or the "nobody" placeholder for `null`.
   * @param userId The user id to label, or `null` for no owner.
   * @returns The label to render.
   * @example
   * ```
   * // private helper; not part of the public API
   * label(null); // t("actors.ownerNobody")
   * ```
   */
  function label(userId: string | null): string {
    if (!userId) return t("actors.ownerNobody");
    return ctx.members.get(userId) ?? userId;
  }

  /**
   * Dispatches a `/owner` Update on the selected token, setting (or clearing, via `null`) the
   * per-token ownership OVERRIDE. This writes the override only — it never touches the linked
   * actor's own `owner` — and does not itself decide who may write `/owner`: that is
   * `cap::EDIT_PERMISSIONS` server-side (`required_cap_for_path`), excluded
   * from the `DocRole::Owner` role's BUILT-IN floor (`role_floor`),
   * so an effective owner (this control's own `resolved` value) cannot write it under that floor
   * alone. Not an absolute: the floored role also selects additive `by_role[Owner]` grants
   * (`effective_role`), so a deployment that puts `EDIT_PERMISSIONS`
   * there would let an effective owner write `/owner` too — nothing in this codebase populates
   * `by_role[Owner]` that way today. A no-op if no token is selected.
   * @param next The user id to set as the override, or `null` to clear it (falling back to the
   * linked actor's owner).
   * @returns Nothing; dispatches an intent as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the owner `<select>`'s `onchange`
   * setOverride("user-1");
   * ```
   */
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
    color: var(--text-muted);
    font-size: 0.85em;
  }
</style>
