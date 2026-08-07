<script lang="ts">
  import { getAppContext } from "./appContext";
  import type { ConflictGroup } from "./mergeConflict";

  let { groups, onApply, onCancel }: {
    groups: ConflictGroup[];
    onApply: (theirsByGroup: Map<string, Set<string>>) => void;
    onCancel: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Radio group name per (group,field). Group keys are opaque document/instance UUIDs
  // and never contain a space, so the join is unambiguous in practice; a NUL-byte join
  // would be formally injective if this ever needs hardening.
  const rowKey = (groupKey: string, path: string): string => `${groupKey} ${path}`;

  // Selection: rowKey → "mine" | "theirs". Default "mine" (keep child).
  let choice = $state<Record<string, "mine" | "theirs">>({});

  let modalEl: HTMLDivElement | undefined;

  $effect(() => {
    modalEl?.focus();
  });

  $effect(() => {
    const onKeydown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  /**
   * Render a conflict side's value for display: an absent (`undefined`) side shows the
   * localized "deleted" label, a string renders verbatim, anything else is JSON-stringified.
   * @param v - The conflict side's value (`c.base`, `c.parent`, or `c.child`).
   * @returns The display string for `v`.
   * @example display(undefined); // t("templates.conflict.deleted")
   */
  function display(v: unknown): string {
    return v === undefined ? t("templates.conflict.deleted") : typeof v === "string" ? v : JSON.stringify(v);
  }

  /**
   * Build the theirs-selection map from the current radio choices and invoke `onApply`.
   * Only paths explicitly chosen `"theirs"` are included per group; a group with no
   * `"theirs"` choices is omitted entirely (its conflicts all resolve to "mine" implicitly).
   * @example apply();
   */
  function apply(): void {
    const out = new Map<string, Set<string>>();
    for (const g of groups) {
      const set = new Set<string>();
      for (const c of g.conflicts) if (choice[rowKey(g.key, c.path)] === "theirs") set.add(c.path);
      if (set.size > 0) out.set(g.key, set);
    }
    onApply(out);
  }
</script>

<div class="modal-scrim" role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal" role="dialog" aria-modal="true" tabindex="-1" aria-label={t("templates.conflict.title")}
       bind:this={modalEl}
       onclick={(e) => e.stopPropagation()}>
    <h2>{t("templates.conflict.title")}</h2>
    {#each groups as g (g.key)}
      {#if g.label !== null}<h3>{g.label}</h3>{/if}
      <ul class="rows">
        {#each g.conflicts as c (c.path)}
          <li class="row">
            <span class="field">{c.path}</span>
            <span class="was">{t("templates.conflict.base")}: {display(c.base)}</span>
            <label>
              <input type="radio" name={rowKey(g.key, c.path)} value="mine"
                     checked={(choice[rowKey(g.key, c.path)] ?? "mine") === "mine"}
                     onchange={() => (choice[rowKey(g.key, c.path)] = "mine")} />
              <span>{t("templates.conflict.mine")}:</span> <span>{display(c.child)}</span>
            </label>
            <label>
              <input type="radio" name={rowKey(g.key, c.path)} value="theirs"
                     checked={choice[rowKey(g.key, c.path)] === "theirs"}
                     onchange={() => (choice[rowKey(g.key, c.path)] = "theirs")} />
              <span>{t("templates.conflict.template")}:</span> <span>{display(c.parent)}</span>
            </label>
          </li>
        {/each}
      </ul>
    {/each}
    <div class="actions">
      <button type="button" class="cancel" onclick={onCancel}>{t("templates.conflict.cancel")}</button>
      <button type="button" class="apply" onclick={apply}>{t("templates.conflict.apply")}</button>
    </div>
  </div>
</div>

<style lang="scss">
  .modal-scrim { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.5); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .modal { background: var(--surface-raised); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-2); padding: var(--space-3); max-width: min(90vw, 40rem); max-height: 85vh; overflow: auto; display: flex; flex-direction: column; gap: var(--space-2); }
  h2 { margin: 0; font-size: var(--font-lg); }
  h3 { margin: var(--space-1) 0 0; font-size: var(--font-md); }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-1); border-bottom: 1px solid var(--border); }
  .field { font-family: monospace; font-weight: 600; }
  .was { opacity: 0.7; }
  label { display: inline-flex; align-items: center; gap: var(--space-1); }
  .actions { display: flex; justify-content: flex-end; gap: var(--space-2); margin-top: var(--space-2); }
  // Touch-target constraint: ≥44px targets under coarse-pointer input.
  button { min-height: 44px; padding: 0 var(--space-3); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface); color: inherit; }
  @media (pointer: coarse) {
    button, input[type="radio"] { min-height: 44px; min-width: 44px; }
  }
  .apply { background: var(--accent); color: var(--accent-contrast, #fff); }
  input:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
