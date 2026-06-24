<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld } from "../api";
  import { t } from "@shadowcat/ui-kit";

  let { onEnter }: { onEnter: (worldId: string) => void } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");
  let error = $state("");

  async function refresh() {
    try {
      worlds = await listWorlds();
    } catch {
      error = t("worlds.errorLoad");
    }
  }
  refresh();

  async function create(e: SubmitEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    error = "";
    try {
      const w = await createWorld(newName.trim());
      newName = "";
      await refresh();
      onEnter(w.id);
    } catch {
      error = t("worlds.errorCreate");
    }
  }
</script>

<main class="entry">
  <h1>{t("worlds.title")}</h1>
  <ul>
    {#each worlds as world (world.id)}
      <li>
        <button onclick={() => onEnter(world.id)}>
          {world.name} <small>({world.role})</small>
        </button>
      </li>
    {/each}
    {#if worlds.length === 0}<li class="empty">{t("worlds.empty")}</li>{/if}
  </ul>
  {#if error}<p role="alert">{error}</p>{/if}
  <form onsubmit={create}>
    <input bind:value={newName} placeholder={t("worlds.newName")} aria-label={t("worlds.newName")} />
    <button type="submit">{t("worlds.create")}</button>
  </form>
</main>

<style lang="scss">
  .entry {
    max-width: 30rem;
    margin: 4rem auto;
    display: grid;
    gap: var(--space-4);
  }
  ul {
    list-style: none;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }
  li button {
    width: 100%;
    text-align: left;
    background: var(--surface-raised);
    border: 1px solid var(--border);
  }
  li button:hover {
    border-color: var(--accent);
    background: var(--surface-overlay);
  }
  .empty {
    color: var(--text-muted);
  }
  form {
    display: flex;
    gap: var(--space-2);
  }
</style>
