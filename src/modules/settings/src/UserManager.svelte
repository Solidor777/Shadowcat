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
      // Clear the plaintext from component state as soon as the request
      // resolves; it is never needed again client-side.
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
