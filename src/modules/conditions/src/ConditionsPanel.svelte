<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildConditionRegistryDoc, conditionTarget, type Condition, type ConditionRegistrySystem, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("condition-registry")[0];
  });
  const conditionEntries = $derived.by((): [string, Condition][] => {
    const sys = registry?.system as ConditionRegistrySystem | undefined;
    return Object.entries(sys?.conditions ?? {});
  });

  // Selected tokens drive the toggle palette: a glyph chip toggles the condition on every
  // selected token whose conditions the current user may edit (GM, or owner via canEdit).
  const selectedTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    return ctx.documents.query("token").filter((tok) => ids.has(tok.id));
  });

  // Idempotent GM seed: a generic emoji set, created once when the registry is absent. The
  // optimistic dispatch adds it to the store immediately, so a second reactive run sees it.
  const SEED: Record<string, Condition> = {
    dead: { name: "Dead", icon: "💀" },
    unconscious: { name: "Unconscious", icon: "😵" },
    prone: { name: "Prone", icon: "🛌" },
    stunned: { name: "Stunned", icon: "💫" },
    poisoned: { name: "Poisoned", icon: "🤢" },
    blinded: { name: "Blinded", icon: "🙈" },
    invisible: { name: "Invisible", icon: "👻" },
    hasted: { name: "Hasted", icon: "⚡" },
    slowed: { name: "Slowed", icon: "🐌" },
  };
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    if (ctx.documents.query("condition-registry").length > 0) {
      seeded = true;
      return;
    }
    seeded = true;
    ctx.dispatchIntent([{ op: "create", doc: buildConditionRegistryDoc(ctx.world, SEED) }]);
  });

  function update(id: string, patch: Partial<Condition>): void {
    if (!registry) return;
    for (const [k, v] of Object.entries(patch)) {
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/conditions/${id}/${k}`, old: null, new: v }] }]);
    }
  }
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const c: Condition = { name: "New condition", icon: "⭐" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/conditions/${id}`, old: null, new: c }] }]);
  }
  function remove(id: string): void {
    const sys = registry?.system as ConditionRegistrySystem | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.conditions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/system/conditions", old: sys.conditions, new: next }] }]);
  }

  /** Whether the condition is set on every editable selected token (chip active state). */
  function isActive(conditionId: string): boolean {
    const targets = selectedTokens.map((tok) => conditionTarget(tok, ctx.documents)).filter((x): x is NonNullable<typeof x> => x !== null);
    return targets.length > 0 && targets.every((tgt) => tgt.conditions.includes(conditionId));
  }

  /** Toggle a condition on each selected token whose conditions the user may edit. When the set
   * is mixed (active on some), this adds it to those missing it; when uniformly on, it clears. */
  function toggle(conditionId: string): void {
    const active = isActive(conditionId);
    for (const tok of selectedTokens) {
      const tgt = conditionTarget(tok, ctx.documents);
      if (!tgt || !ctx.canEdit(tgt.doc, tgt.path)) continue;
      const has = tgt.conditions.includes(conditionId);
      if (active === has) {
        const next = has ? tgt.conditions.filter((c) => c !== conditionId) : [...tgt.conditions, conditionId];
        ctx.dispatchIntent([{ op: "update", doc_id: tgt.doc.id, changes: [{ path: tgt.path, old: tgt.conditions, new: next }] }]);
      }
    }
  }
</script>

<section class="conditions">
  <h3>{t("conditions.title")}</h3>

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

  {#if ctx.role === "gm"}
    <ul class="list">
      {#each conditionEntries as [id, c] (id)}
        <li>
          <span class="glyph">{c.icon}</span>
          <input aria-label={t("conditions.name")} value={c.name} onchange={(e) => update(id, { name: e.currentTarget.value })} />
          <input aria-label={t("conditions.icon")} value={c.icon} maxlength="4" onchange={(e) => update(id, { icon: e.currentTarget.value })} />
          <button type="button" onclick={() => remove(id)}>{t("conditions.remove")}</button>
        </li>
      {/each}
    </ul>
    <button type="button" onclick={add}>{t("conditions.add")}</button>
  {/if}
</section>

<style lang="scss">
  .conditions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.85em;
  }
  .palette {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .palette button {
    min-width: 36px;
    min-height: 36px;
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
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .glyph {
    font-size: 1.1em;
    flex: 0 0 auto;
  }
  input,
  button {
    min-height: 32px;
  }
</style>
