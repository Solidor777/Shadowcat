<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveTokenActor, type Faction, type FactionRegistryEngine, type WireDocument } from "@shadowcat/core";
  import { seedFactionRegistryIfAbsent } from "./seed";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("faction-registry")[0];
  });
  const factionEntries = $derived.by((): [string, Faction][] => {
    const sys = registry?.engine as FactionRegistryEngine | undefined;
    return Object.entries(sys?.factions ?? {});
  });

  // Idempotent GM seed: create the registry (deterministic id, so racing GMs converge on one)
  // once, only when absent. The optimistic dispatch adds it to the store immediately, so a
  // second reactive run sees it and `seeded` short-circuits further attempts.
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    seeded = true;
    seedFactionRegistryIfAbsent(ctx.documents, ctx.world, ctx.dispatchIntent);
  });

  /** GM registry editor: patches one or more fields of a faction entry, dispatching one
   * `update` op per changed field. `old` reads the RAW currently-stored value (never a
   * resolved/defaulted one) — the server's field-level optimistic-concurrency check
   * (`apply_intent`) rejects an `Update` whose `old` doesn't match the actual stored value,
   * so a hardcoded `old: null` would only be valid for the field's first write.
   * @param id The faction's registry key.
   * @param patch The fields to change (name, color, and/or stance).
   * @example
   * ```
   * // private function; not part of the public API — invoked from the GM editor row's
   * // name/color/stance inputs
   * update("hostile", { name: "Hostile" });
   * ```
   */
  function update(id: string, patch: Partial<Faction>): void {
    if (!registry) return;
    // `old` must be the field's REAL current stored value (or null when genuinely absent): the
    // server's apply_intent enforces field-level OCC (actual != change.old -> Conflict), so a
    // hardcoded `old: null` is only valid once and is rejected on every subsequent edit once the
    // field holds a non-null value (mirrors GameSettingsPanel's `set` helper fix).
    const eng = registry.engine as FactionRegistryEngine;
    const current = eng.factions[id] as Partial<Faction> | undefined;
    for (const [k, v] of Object.entries(patch)) {
      const old = current?.[k as keyof Faction] ?? null;
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/factions/${id}/${k}`, old, new: v }] }]);
    }
  }
  /** GM registry editor: appends a new faction entry under a fresh random id, with a
   * placeholder name/color and neutral stance for the GM to rename in place.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "Add" button
   * add();
   * ```
   */
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const f: Faction = { name: "New faction", color: "#9e9e9e", stance: "neutral" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/factions/${id}`, old: null, new: f }] }]);
  }
  /** GM registry editor: deletes a faction entry from the registry map.
   * @param id The faction's registry key to remove.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each row's remove button
   * remove("hostile");
   * ```
   */
  function remove(id: string): void {
    const sys = registry?.engine as FactionRegistryEngine | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.factions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/engine/factions", old: sys.factions, new: next }] }]);
  }
  /** GM registry editor: replaces the current token selection with every scene token whose
   * effective actor (`resolveTokenActor`) is assigned to `factionId` — a read-only selection
   * change, no document write, so it needs no `canEdit`/GM gate of its own beyond the
   * surrounding `{#if ctx.role === "gm"}` template block this button lives in.
   * @param factionId The faction's registry key to select tokens by.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each row's "select tokens"
   * // button
   * selectTokens("hostile");
   * ```
   */
  function selectTokens(factionId: string): void {
    const ids = ctx.documents.query("token").filter((tok) => resolveTokenActor(tok, ctx.documents)?.faction === factionId).map((tok) => tok.id);
    ctx.tokenSelection.set(ids);
  }
</script>

<section class="factions">
  <h3>{t("factions.title")}</h3>
  <ul class="list">
    {#each factionEntries as [id, f] (id)}
      <li>
        <span class="swatch" style="background:{f.color}"></span>
        {#if ctx.role === "gm"}
          <input aria-label={t("factions.name")} value={f.name} onchange={(e) => update(id, { name: e.currentTarget.value })} />
          <input type="color" aria-label={t("factions.color")} value={f.color} onchange={(e) => update(id, { color: e.currentTarget.value })} />
          <select aria-label={t("factions.stance")} value={f.stance} onchange={(e) => update(id, { stance: e.currentTarget.value as Faction["stance"] })}>
            <option value="friendly">{t("factions.friendly")}</option>
            <option value="neutral">{t("factions.neutral")}</option>
            <option value="hostile">{t("factions.hostile")}</option>
          </select>
          <button type="button" onclick={() => remove(id)}>{t("factions.remove")}</button>
          <button type="button" onclick={() => selectTokens(id)}>{t("factions.selectTokens")}</button>
        {:else}
          <span>{f.name}</span>
        {/if}
      </li>
    {/each}
  </ul>
  {#if ctx.role === "gm"}
    <button type="button" onclick={add}>{t("factions.add")}</button>
  {/if}
</section>

<style lang="scss">
  .factions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
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
  .swatch {
    width: 16px;
    height: 16px;
    border-radius: var(--radius-1);
    border: 1px solid var(--border);
    flex: 0 0 auto;
  }
  input,
  select,
  button {
    min-height: 32px;
  }
</style>
