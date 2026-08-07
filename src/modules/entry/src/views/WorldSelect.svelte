<script lang="ts">
  import type { WorldEntry } from "@shadowcat/types";
  import { listWorlds, createWorld, acceptInvite, deleteWorld } from "../entryApi";
  import { t } from "@shadowcat/ui-kit";

  let {
    onEnter,
  }: {
    /** Invoked to enter a world, whether picked from the list, just created, or just
     * redeemed via invite — `Entry` hands this straight through to the shell. */
    onEnter: (worldId: string) => void;
  } = $props();
  let worlds = $state<WorldEntry[]>([]);
  let newName = $state("");
  let inviteCode = $state("");
  let error = $state("");
  let confirmingDelete = $state<string | null>(null);
  let deleteName = $state("");
  let deleting = $state(false);

  /**
   * Toggle `id`'s type-the-exact-name delete-confirmation row: disarms it if
   * already armed, otherwise arms it (replacing any other world's armed row —
   * only one confirmation row is open at a time). Clears the typed name and
   * any error either way.
   * @param id The world id whose delete-confirmation row is being toggled.
   * @example
   * ```
   * // module-private; not part of the public API — bound to the delete button
   * armDelete(world.id);
   * ```
   */
  function armDelete(id: string) {
    confirmingDelete = confirmingDelete === id ? null : id;
    deleteName = "";
    error = "";
  }

  /**
   * Delete `world`, guarded by two independent legs: the typed name must
   * match `world.name` exactly (`deleteName`, armed by `armDelete`), AND
   * `deleting` re-entrancy-guards against a second submit while the request
   * is in flight. Either leg failing is a silent no-op — this is the only
   * confirmation guard in the delete path; `deleteWorld` itself performs
   * none. On failure, the thrown message is discarded in favor of a generic
   * error (`worlds.errorDelete`).
   * @param world The world to delete, matched against the typed confirmation.
   * @example
   * ```
   * // module-private; not part of the public API — bound to the confirm form
   * confirmDelete(world);
   * ```
   */
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

  /**
   * (Re)load the world list. On failure, the thrown message is discarded in
   * favor of a generic error (`worlds.errorLoad`); `worlds` is left at its
   * previous value.
   * @example
   * ```
   * // module-private; not part of the public API — called on mount and after
   * // create/delete/redeem
   * refresh();
   * ```
   */
  async function refresh() {
    try {
      worlds = await listWorlds();
    } catch {
      error = t("worlds.errorLoad");
    }
  }
  refresh();

  /**
   * Handle the create-world form submit: create the world, refresh the list,
   * then enter it. On failure, the thrown message is discarded in favor of a
   * generic error (`worlds.errorCreate`).
   * @param e The form's submit event.
   * @example
   * ```
   * // module-private; not part of the public API — bound to <form onsubmit>
   * create(event);
   * ```
   */
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

  /**
   * Handle the invite-redemption form submit. Unlike `refresh`/`create`/
   * `confirmDelete`, this has no `try`/`catch`: `acceptInvite` collapses every
   * HTTP-level rejection to `null` one layer down, so there is no thrown
   * message to discard. See `acceptInvite`'s doc — the
   * statement of record for the no-oracle rationale, and the only place that
   * enumerates the rejection cases. The generic error shown here
   * (`worlds.errorRedeem`) reflects that collapse; it is not a choice this
   * function makes.
   *
   * A network-level `fetch` rejection is NOT covered: neither `postJson` nor
   * `acceptInvite` catches one, so it propagates out of this handler as an
   * unhandled rejection — where the three `catch` paths above would absorb it.
   * @param e The form's submit event.
   * @example
   * ```
   * // module-private; not part of the public API — bound to <form onsubmit>
   * redeem(event);
   * ```
   */
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
