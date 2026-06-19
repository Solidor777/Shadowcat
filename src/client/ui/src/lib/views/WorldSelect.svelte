<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld } from "../api";

  let { onEnter }: { onEnter: (worldId: string) => void } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");
  let error = $state("");

  async function refresh() {
    try {
      worlds = await listWorlds();
    } catch {
      error = "Could not load worlds.";
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
      error = "Could not create world.";
    }
  }
</script>

<main class="entry">
  <h1>Your worlds</h1>
  <ul>
    {#each worlds as world (world.id)}
      <li>
        <button onclick={() => onEnter(world.id)}>
          {world.name} <small>({world.role})</small>
        </button>
      </li>
    {/each}
    {#if worlds.length === 0}<li class="empty">No worlds yet.</li>{/if}
  </ul>
  {#if error}<p role="alert">{error}</p>{/if}
  <form onsubmit={create}>
    <input bind:value={newName} placeholder="New world name" aria-label="New world name" />
    <button type="submit">Create world</button>
  </form>
</main>

<style>
  .entry { max-width: 30rem; margin: 4rem auto; display: grid; gap: 1rem; }
  ul { list-style: none; padding: 0; display: grid; gap: 0.5rem; }
  form { display: flex; gap: 0.5rem; }
</style>
