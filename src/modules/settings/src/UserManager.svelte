<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { listUsers, createUser, deleteUser, type ServerUser } from "@shadowcat/core";

  // Server-administration surface: creating accounts and listing them are both
  // gated on ServerRole::Admin server-side. Rendering is gated on `serverRole`
  // (never `role` — a world GM is not a server admin), which is advisory only.
  const { t, serverRole, selfId } = getAppContext();

  let users = $state<ServerUser[]>([]);
  let loaded = $state(false);
  let username = $state("");
  let password = $state("");
  let makeAdmin = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let created = $state<string | null>(null);
  let deleteError = $state<string | null>(null);

  /**
   * Deletes user `u` after an explicit confirm, then reloads the user list.
   * Reports failure on its OWN channel, `deleteError` — kept separate from
   * `load()`'s and `submit()`'s shared `error`, so a failed delete never
   * gets papered over by (or itself clears) an unrelated create/load
   * failure still worth showing. Declining the confirm is a true no-op:
   * every state field is left untouched.
   * @param u The user row to delete; only `id`/`username` are read.
   * @returns Resolves once `deleteError`/`users` (via `load()`) reflect the
   *   outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's delete button
   * declare const user: ServerUser;
   * await removeUser(user);
   * ```
   */
  async function removeUser(u: ServerUser): Promise<void> {
    if (!window.confirm(t("settings.users.deleteConfirm", { username: u.username }))) return;
    busy = true;
    deleteError = null;
    try {
      await deleteUser(u.id);
      await load();
    } catch (e) {
      deleteError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  /**
   * Loads the full user list. A single read, unlike `InviteManager.refresh`
   * and `ModuleManager.load`'s independent pairs — there is no
   * partial-result question here: the one call either replaces `users`
   * wholesale or leaves it at its prior value while `error` reports why.
   * @returns Resolves once `users`/`error`/`loaded` reflect the outcome;
   *   never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked at mount (admin only)
   * // and after create/delete
   * await load();
   * ```
   */
  async function load(): Promise<void> {
    error = null;
    try {
      users = await listUsers();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }
  if (serverRole === "admin") void load();

  /**
   * Creates a user account, then reloads the user list. On SUCCESS, clears
   * `username`/`password`/`makeAdmin` from component state immediately after
   * the request resolves — none is needed again client-side. On FAILURE
   * (the `catch`), none of the three is cleared: the form keeps whatever the
   * admin typed, including the plaintext password, so a rejected submit
   * (e.g. a duplicate username) doesn't force retyping it. Clearing a
   * `$state` field only drops THIS component's own reference to the string;
   * it does not guarantee the value is unrecoverable from the JS runtime's
   * memory (strings are immutable, and the prior value may live until GC) —
   * treat this as removing it from the rendered/bound surface, not as a
   * secure-erase guarantee.
   * @param e The form's submit event; only `preventDefault` is used.
   * @returns Resolves once `created`/`error`/`users` (via `load()`) reflect
   *   the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the create-user form's onsubmit;
   * // reads username/password/makeAdmin from component state, not from this event
   * declare const submitEvent: SubmitEvent;
   * await submit(submitEvent);
   * ```
   */
  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    busy = true;
    error = null;
    created = null;
    try {
      const user = await createUser({
        username,
        password,
        serverRole: makeAdmin ? "admin" : "user",
      });
      created = user.username;
      // Success path only — see this function's JSDoc for what a rejected
      // submit leaves in place.
      username = "";
      password = "";
      makeAdmin = false;
      await load();
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

{#if serverRole === "admin"}
  <section class="user-manager">
    <h3>{t("settings.users.title")}</h3>
    <form onsubmit={submit}>
      <label>
        {t("settings.users.username")}
        <input bind:value={username} autocomplete="off" required />
      </label>
      <label>
        {t("settings.users.password")}
        <input type="password" bind:value={password} autocomplete="new-password" required />
      </label>
      <label class="inline">
        <input type="checkbox" bind:checked={makeAdmin} />
        {t("settings.users.makeAdmin")}
      </label>
      <button type="submit" disabled={busy}>{t("settings.users.create")}</button>
    </form>
    {#if created}
      <p class="ok">{t("settings.users.created", { username: created })}</p>
    {/if}
    {#if error}
      <p class="error">{t("settings.users.error", { message: error })}</p>
    {/if}
    {#if deleteError}
      <p class="error">{t("settings.users.deleteError", { message: deleteError })}</p>
    {/if}
    {#if !loaded}
      <p>{t("settings.users.loading")}</p>
    {:else}
      <ul>
        {#each users as u (u.id)}
          <li>
            <span>{u.username} <span class="tier">{u.server_role}</span></span>
            {#if u.id !== selfId}
              <button class="danger-outline" onclick={() => removeUser(u)} disabled={busy}>
                {t("settings.users.delete")}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style lang="scss">
  .user-manager {
    display: grid;
    gap: var(--space-2);
  }
  form {
    display: grid;
    gap: var(--space-2);
  }
  .inline {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
  .tier {
    color: var(--text-muted);
  }
  ul {
    list-style: none;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }
  li {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }
  button.danger-outline {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
  }
  @media (pointer: coarse) {
    button.danger-outline {
      min-height: var(--input-height-coarse);
    }
  }
  input {
    min-height: 32px;
  }
  input[type="checkbox"] {
    min-width: 36px;
    min-height: 36px;
  }
  button {
    min-height: 32px;
  }
</style>
