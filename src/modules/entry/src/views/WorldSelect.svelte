<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld, acceptInvite, deleteWorld } from "../entryApi";
  import { t } from "@shadowcat/ui-kit";

  let { onEnter }: { onEnter: (worldId: string) => void } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");
  let inviteCode = $state("");
  let error = $state("");
  let confirmingDelete = $state<string | null>(null);
  let deleteName = $state("");
  let deleting = $state(false);

  function armDelete(id: string) {
    confirmingDelete = confirmingDelete === id ? null : id;
    deleteName = "";
    error = "";
  }

  async function confirmDelete(world: WorldEntry) {
    if (deleteName !== world.name || deleting) return;
    deleting = true;
    error = "";
    try {
      await deleteWorld(world.id);
      confirmingDelete = null;
      deleteName = "";
      await refresh();
    } catch {
      error = t("worlds.errorDelete");
    } finally {
      deleting = false;
    }
  }

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
        <div class="row">
          <button class="enter" onclick={() => onEnter(world.id)}>
            {world.name} <small>({world.role})</small>
          </button>
          {#if world.role === "gm"}
            <button class="danger-outline" onclick={() => armDelete(world.id)}>
              {t("worlds.delete")}
            </button>
          {/if}
        </div>
        {#if confirmingDelete === world.id}
          <form
            class="confirm-delete"
            onsubmit={(e) => {
              e.preventDefault();
              confirmDelete(world);
            }}
          >
            <label>
              {t("worlds.deleteTypeName")}
              <input bind:value={deleteName} placeholder={world.name} />
            </label>
            <button type="submit" class="danger" disabled={deleteName !== world.name || deleting}>
              {t("worlds.deleteConfirm")}
            </button>
          </form>
        {/if}
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
  li .row {
    display: flex;
    gap: var(--space-2);
    align-items: stretch;
  }
  li .row .enter {
    flex: 1;
    text-align: left;
    background: var(--surface-raised);
    border: 1px solid var(--border);
  }
  li .row .enter:hover {
    border-color: var(--accent);
    background: var(--surface-overlay);
  }
  .confirm-delete {
    display: flex;
    gap: var(--space-2);
    align-items: end;
    margin-top: var(--space-2);
  }
  button.danger-outline {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
  }
  button.danger {
    background: var(--danger);
    border: 1px solid var(--danger);
    color: var(--on-danger);
  }
  button.danger:disabled {
    opacity: 0.5;
  }
  @media (pointer: coarse) {
    button.danger,
    button.danger-outline {
      min-height: var(--input-height-coarse);
    }
  }
  .empty {
    color: var(--text-muted);
  }
  form {
    display: flex;
    gap: var(--space-2);
  }
</style>
