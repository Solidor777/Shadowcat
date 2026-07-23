<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld, acceptInvite } from "../entryApi";
  import { t } from "@shadowcat/ui-kit";

  let { onEnter }: { onEnter: (worldId: string) => void } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");
  let inviteCode = $state("");
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

  // Redeeming seats the caller in the invite's world. The server answers every
  // unusable code identically, so this reports one generic failure — inferring
  // a reason would re-create the oracle the invite flow exists to remove.
  async function redeem(e: SubmitEvent) {
    e.preventDefault();
    if (!inviteCode.trim()) return;
    error = "";
    const world = await acceptInvite(inviteCode.trim());
    if (!world) {
      error = t("worlds.errorRedeem");
      return;
    }
    inviteCode = "";
    await refresh();
    onEnter(world.id);
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
  <form onsubmit={redeem}>
    <input
      bind:value={inviteCode}
      placeholder={t("worlds.redeemCode")}
      aria-label={t("worlds.redeemCode")}
      autocomplete="off"
    />
    <button type="submit">{t("worlds.redeem")}</button>
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
