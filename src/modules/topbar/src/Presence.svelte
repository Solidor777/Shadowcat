<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;

  // `members` is a reactive SvelteMap (userId -> username), populated for every
  // role (M11d-1). Reading it here tracks join/leave updates in place.
  const roster = $derived([...ctx.members.entries()].map(([id, name]) => ({ id, name })));

  function initial(name: string): string {
    return name.trim().charAt(0).toUpperCase() || "?";
  }
</script>

<div class="sc-presence" role="group" aria-label={t("topbar.presence")} data-testid="presence">
  {#each roster as m (m.id)}
    <span
      class="sc-presence-badge"
      title={m.name}
      aria-label={m.name}
      data-testid="presence-{m.id}">{initial(m.name)}</span>
  {/each}
</div>

<style lang="scss">
  .sc-presence {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    overflow: hidden;
  }
  .sc-presence-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 999px;
    background: var(--surface-overlay);
    border: 1px solid var(--border);
    color: var(--text-primary);
    font-size: 0.75rem;
    line-height: 1;
    flex: 0 0 auto;
  }
</style>
