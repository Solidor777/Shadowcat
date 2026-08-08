<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildConditionRegistryDoc, conditionTarget, type Condition, type ConditionRegistryEngine, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("condition-registry")[0];
  });
  const conditionEntries = $derived.by((): [string, Condition][] => {
    const sys = registry?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(sys?.conditions ?? {});
  });

  // Selected tokens drive the toggle palette: a glyph chip toggles the condition on every
  // selected token whose conditions the current user may edit (GM, or owner via canEdit).
  const selectedTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    return ctx.documents.query("token").filter((tok) => ids.has(tok.id));
  });

  // Idempotent GM seed: a generic emoji set, created once when the registry is absent — this
  // default content is replaceable, per the `conditions` module's own modding contract: a
  // game-system module can supply its own seed/editor instead. The optimistic dispatch adds it
  // to the store immediately, so a second reactive run sees it and this effect no-ops.
  // Divergence from the sibling faction-registry seed (`factions`'s `seedFactionRegistryIfAbsent`):
  // that seed dispatches its Create under a `deterministicId(worldId, ...)` id so
  // two racing GMs converge on the same id; this one calls `buildConditionRegistryDoc` with no
  // explicit id, so two racing GMs' Creates carry two DIFFERENT random ids. This is still
  // correctness-safe — CONDITION_REGISTRY_DOC_TYPE is in the server's doc_type-scoped
  // `data::sqlite`'s `SINGLETON_DOC_TYPES` list, so the loser's Create is rejected regardless
  // of id, and `OptimisticClient.reject` rolls its local prediction back the same way — but it
  // means this seed doesn't need (and doesn't use) the id-convergence property the faction
  // seed's own doc comment relies on.
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

  /** GM registry editor: patches one or more fields of a condition entry, dispatching one
   * `update` op per changed field. `old` reads the RAW currently-stored value (never a
   * resolved/defaulted one) — the server's field-level optimistic-concurrency check
   * (`apply_intent`) rejects an `Update` whose `old` doesn't match the actual stored value,
   * so a hardcoded `old: null` would only be valid for the field's first write.
   * @param id The condition's registry key.
   * @param patch The fields to change (name and/or icon).
   * @example
   * ```
   * // private function; not part of the public API — invoked from the GM editor row's
   * // name/icon inputs
   * update("prone", { name: "Prone" });
   * ```
   */
  function update(id: string, patch: Partial<Condition>): void {
    if (!registry) return;
    // `old` must be the field's REAL current stored value (or null when genuinely absent): the
    // server's apply_intent enforces field-level OCC (actual != change.old -> Conflict), so a
    // hardcoded `old: null` is only valid once and is rejected on every subsequent edit once the
    // field holds a non-null value (mirrors GameSettingsPanel's `set` helper fix).
    const eng = registry.engine as ConditionRegistryEngine;
    const current = eng.conditions[id] as Partial<Condition> | undefined;
    for (const [k, v] of Object.entries(patch)) {
      const old = current?.[k as keyof Condition] ?? null;
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/conditions/${id}/${k}`, old, new: v }] }]);
    }
  }
  /** GM registry editor: appends a new condition entry under a fresh random id, with a
   * placeholder name/icon for the GM to rename in place.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "Add" button
   * add();
   * ```
   */
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const c: Condition = { name: "New condition", icon: "⭐" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/conditions/${id}`, old: null, new: c }] }]);
  }
  /** GM registry editor: deletes a condition entry from the registry map.
   * @param id The condition's registry key to remove.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each row's remove button
   * remove("prone");
   * ```
   */
  function remove(id: string): void {
    const sys = registry?.engine as ConditionRegistryEngine | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.conditions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/engine/conditions", old: sys.conditions, new: next }] }]);
  }

  /** Selection-driven toggle palette: whether `conditionId` is set on every selected token
   * that resolves a conditions target at all (`conditionTarget` — a token whose actor link is
   * dangling, or that has neither a link nor an embedded actor, is excluded). Unlike `toggle`,
   * this does NOT filter by `canEdit` — a token the current user cannot edit still counts
   * toward the "every token has it" check, so the palette's active/mixed chip state can
   * reflect a token whose own condition a click on the chip would not actually change.
   * @param conditionId The registry key to check.
   * @returns `true` only when at least one target-resolving selected token exists and every
   * one of them currently has the condition set.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the palette chip's
   * // class:active/aria-pressed bindings
   * isActive("prone");
   * ```
   */
  function isActive(conditionId: string): boolean {
    const targets = selectedTokens.map((tok) => conditionTarget(tok, ctx.documents)).filter((x): x is NonNullable<typeof x> => x !== null);
    return targets.length > 0 && targets.every((tgt) => tgt.conditions.includes(conditionId));
  }

  /** Selection-driven toggle palette: toggles a condition across the selected tokens whose
   * conditions the current user may edit (`AppContext.canEdit`, GM or token owner — a
   * client-advisory gate only; the server independently re-checks each Update). Direction is
   * decided by `isActive`'s BROADER verdict (which also counts non-editable selected tokens):
   * when `isActive` reports uniformly active, this REMOVES the condition from each editable
   * token that currently has it; otherwise (mixed or none), this ADDS it to each editable
   * token currently missing it — an editable token that already has it is left untouched in
   * that second branch. A non-editable token is never mutated either way, only (potentially)
   * counted by `isActive` when deciding which of the two directions to take.
   * @param conditionId The registry key to toggle.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the palette chip's onclick
   * toggle("prone");
   * ```
   */
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
