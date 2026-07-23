<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { addWorldMemberByUsername } from "@shadowcat/core";

  /** The world tier's closed role set, mirroring the server's `WorldRole`.
   * Structurally assignable to it — a server-tier value is not expressible. */
  type WorldRole = "player" | "spectator" | "gm";

  // GM surface: seat an existing account in this world by the username the
  // administrator issued. Deliberately name-driven — a GM is never handed a
  // server-wide user directory (`/api/users` is admin-only), and the route
  // accepts only a WorldRole, so nothing here can grant server administration.
  const { t, role, world, members } = getAppContext();

  const ROLES: WorldRole[] = ["player", "spectator", "gm"];

  let username = $state("");
  let worldRole = $state<WorldRole>("player");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let seated = $state<string | null>(null);

  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    busy = true;
    error = null;
    seated = null;
    try {
      await addWorldMemberByUsername(world, username.trim(), worldRole);
      seated = username.trim();
      username = "";
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

{#if role === "gm"}
  <section class="member-manager">
    <h3>{t("settings.members.title")}</h3>
    <form onsubmit={submit}>
      <label>
        {t("settings.members.username")}
        <input bind:value={username} autocomplete="off" required />
      </label>
      <label>
        {t("settings.members.role")}
        <select bind:value={worldRole}>
          {#each ROLES as r (r)}<option value={r}>{r}</option>{/each}
        </select>
      </label>
      <button type="submit" disabled={busy}>{t("settings.members.add")}</button>
    </form>
    {#if seated}
      <p class="ok">{t("settings.members.added", { username: seated })}</p>
    {/if}
    {#if error}
      <p class="error">{t("settings.members.error", { message: error })}</p>
    {/if}
    <ul>
      {#each [...members.values()] as name (name)}
        <li>{name}</li>
      {/each}
    </ul>
  </section>
{/if}

<style lang="scss">
  .member-manager {
    display: grid;
    gap: var(--space-2);
  }
  form {
    display: grid;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
  input,
  select {
    min-height: 32px;
  }
  button {
    min-height: 32px;
  }
</style>
