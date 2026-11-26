<script lang="ts">
  // Selection-driven condition toggle palette: a glyph chip per registry condition toggles that
  // condition on every selected token whose conditions the current user may edit. Mounted by
  // both the conditions panel and the token quick-edit section, so the displayed state and the
  // click's mutated set come from this one component.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "./appContext";
  import { conditionTarget, buildUpdate, type Condition, type ConditionRegistryEngine, type ConditionTarget, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const conditionEntries = $derived.by((): [string, Condition][] => {
    subscribe();
    const sys = ctx.documents.query("condition-registry")[0]?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(sys?.conditions ?? {});
  });

  const selectedTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    return ctx.documents.query("token").filter((tok) => ids.has(tok.id));
  });

  /** The canEdit-gated target set both `isActive` and `toggle` read: every selected token that
   * resolves a conditions target (`conditionTarget` — a token whose actor link is dangling, or
   * that has neither a link nor an embedded actor, is excluded) AND whose target the current
   * user may edit (`AppContext.canEdit`, GM or token owner — a client-advisory gate only; the
   * server independently re-checks each Update). A single shared symbol, so the palette's
   * displayed state and its click's mutated set can never diverge on which tokens count.
   * @returns Every editable conditions target in the current selection.
   * @example
   * ```
   * // private function; not part of the public API
   * const targets = editableTargets();
   * ```
   */
  function editableTargets(): ConditionTarget[] {
    return selectedTokens
      .map((tok) => conditionTarget(tok, ctx.documents))
      .filter((x): x is NonNullable<typeof x> => x !== null)
      .filter((tgt) => ctx.canEdit(tgt.doc, tgt.path));
  }

  /** Whether `conditionId` is set on every `editableTargets` entry — the same canEdit-gated set
   * `toggle` mutates, so the palette's active/mixed chip state always reflects what a click can
   * actually change.
   * @param conditionId The registry key to check.
   * @returns `true` only when at least one editable target exists and every one of them
   * currently has the condition set.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the chip's
   * // class:active/aria-pressed bindings
   * isActive("prone");
   * ```
   */
  function isActive(conditionId: string): boolean {
    const targets = editableTargets();
    return targets.length > 0 && targets.every((tgt) => tgt.conditions.includes(conditionId));
  }

  /** Toggles a condition across `editableTargets` — the same set `isActive` reads. Direction is
   * decided by `isActive`'s verdict over that identical set: when uniformly active, this REMOVES
   * the condition from each target that currently has it; otherwise (mixed or none), this ADDS it
   * to each target currently missing it — a target that already has it is left untouched in that
   * second branch.
   * @param conditionId The registry key to toggle.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the chip's onclick
   * toggle("prone");
   * ```
   */
  function toggle(conditionId: string): void {
    const active = isActive(conditionId);
    for (const tgt of editableTargets()) {
      const has = tgt.conditions.includes(conditionId);
      if (active === has) {
        const next = has ? tgt.conditions.filter((c) => c !== conditionId) : [...tgt.conditions, conditionId];
        ctx.dispatchIntent([buildUpdate(tgt.doc.id, [{ path: tgt.path, old: tgt.conditions, value: next }])]);
      }
    }
  }
</script>

{#if selectedTokens.length > 0}
  <p class="hint">{t("conditions.toggleHint")}</p>
  <div class="palette">
    {#each conditionEntries as [id, c] (id)}
      <button type="button" class:active={isActive(id)} aria-pressed={isActive(id)} title={c.name} onclick={() => toggle(id)}>{c.icon}</button>
    {/each}
  </div>
{:else}
  <p class="hint">{t("conditions.selectHint")}</p>
{/if}

<style lang="scss">
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 0.85em;
  }
  .palette {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .palette button {
    min-width: 44px;
    min-height: 44px;
    font-size: 1.1em;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    cursor: pointer;
  }
  .palette button.active {
    border-color: var(--accent);
    background: var(--accent);
  }
</style>
