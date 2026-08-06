<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;

  // A reactive SvelteMap (userId -> username), populated for every role
  // (M11d-1). `WorldSession.#onWelcome` refreshes it only on a WS (re)connect
  // Welcome, not on each individual join/leave — a member seated mid-session
  // does not appear here until the next reconnect. Reading it via $derived does track whatever
  // updates DO land (the map is mutated in place, never reassigned), so this
  // badge list repaints once a reconnect's fetch resolves; it does not
  // repaint the instant a seat is actually granted.
  const roster = $derived([...ctx.members.entries()].map(([id, name]) => ({ id, name })));

  /**
   * Single-character badge glyph for a member name: the trimmed name's first
   * character, uppercased, or `"?"` for an empty/whitespace-only name.
   * @param name The member's display name.
   * @returns A one-character (or `"?"`) badge glyph.
   * @example
   * ```
   * // private function; not part of the public API — used to render each presence badge
   * initial("Zara");
   * ```
   */
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
