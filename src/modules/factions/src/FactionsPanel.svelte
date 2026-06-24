<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildFactionRegistryDoc, type Faction, type FactionRegistrySystem, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("faction-registry")[0];
  });
  const factionEntries = $derived.by((): [string, Faction][] => {
    const sys = registry?.system as FactionRegistrySystem | undefined;
    return Object.entries(sys?.factions ?? {});
  });

  // Idempotent GM seed: create the registry with three defaults once, only when absent. The
  // optimistic dispatch adds it to the store immediately, so a second reactive run sees it.
  const SEED: Record<string, Faction> = {
    friendly: { name: "Friendly", color: "#3fb950", stance: "friendly" },
    neutral: { name: "Neutral", color: "#9e9e9e", stance: "neutral" },
    hostile: { name: "Hostile", color: "#f85149", stance: "hostile" },
  };
  let seeded = false;
  $effect(() => {
    if (ctx.role !== "gm" || seeded) return;
    subscribe();
    if (ctx.documents.query("faction-registry").length > 0) {
      seeded = true;
      return;
    }
    seeded = true;
    ctx.dispatchIntent([{ op: "create", doc: buildFactionRegistryDoc(ctx.world, SEED) }]);
  });

  function update(id: string, patch: Partial<Faction>): void {
    if (!registry) return;
    for (const [k, v] of Object.entries(patch)) {
      ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/factions/${id}/${k}`, old: null, new: v }] }]);
    }
  }
  function add(): void {
    if (!registry) return;
    const id = crypto.randomUUID();
    const f: Faction = { name: "New faction", color: "#9e9e9e", stance: "neutral" };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/system/factions/${id}`, old: null, new: f }] }]);
  }
  function remove(id: string): void {
    const sys = registry?.system as FactionRegistrySystem | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.factions };
    delete next[id];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/system/factions", old: sys.factions, new: next }] }]);
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
