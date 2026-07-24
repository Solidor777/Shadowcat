<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    createWorldInvite,
    listWorldInvites,
    revokeWorldInvite,
    listWorldMembers,
    type InviteEntry,
    type WorldMember,
  } from "@shadowcat/core";

  /** The world tier's closed role set, mirroring the server's `WorldRole`.
   * Structurally assignable to it — a server-tier value is not expressible. */
  type WorldRole = "player" | "spectator" | "gm";

  // GM surface: mint a single-use invite, hand the code to the player out of
  // band, revoke it if it goes astray. The GM never names an account — naming
  // one would make the membership route a username-existence oracle — so the
  // invited user redeems the code from their own session.
  const { t, role, world } = getAppContext();

  const ROLES: WorldRole[] = ["player", "spectator", "gm"];

  let worldRole = $state<WorldRole>("player");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let invites = $state<InviteEntry[]>([]);
  let members = $state<WorldMember[]>([]);
  // The code is shown exactly once: the server stores only a hash of it, so it
  // is unrecoverable after this render.
  let minted = $state<string | null>(null);

  // Both lists are re-read from the server on every state change this surface
  // causes. The roster in particular must NOT come from AppContext's `members`
  // map: that is a session-start snapshot, so a seat added during the session
  // would never appear.
  async function refresh(): Promise<void> {
    try {
      [invites, members] = await Promise.all([
        listWorldInvites(world),
        listWorldMembers(world),
      ]);
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }
  if (role === "gm") void refresh();

  async function mint(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    busy = true;
    error = null;
    minted = null;
    try {
      minted = (await createWorldInvite(world, worldRole)).code;
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }

  async function revoke(id: string): Promise<void> {
    error = null;
    try {
      await revokeWorldInvite(world, id);
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  function status(i: InviteEntry): string {
    if (i.consumed_at !== null) return t("settings.invites.consumed");
    if (i.revoked_at !== null) return t("settings.invites.revoked");
    if (i.expires_at <= Date.now()) return t("settings.invites.expired");
    return t("settings.invites.active");
  }

  function spent(i: InviteEntry): boolean {
    return i.consumed_at !== null || i.revoked_at !== null;
  }
</script>

{#if role === "gm"}
  <section class="invite-manager">
    <h3>{t("settings.invites.title")}</h3>
    <form onsubmit={mint}>
      <label>
        {t("settings.invites.role")}
        <select bind:value={worldRole}>
          {#each ROLES as r (r)}<option value={r}>{r}</option>{/each}
        </select>
      </label>
      <button type="submit" disabled={busy}>{t("settings.invites.mint")}</button>
    </form>
    {#if minted}
      <p class="ok">{t("settings.invites.minted")}</p>
      <input class="code" readonly value={minted} aria-label={t("settings.invites.code")} />
    {/if}
    {#if error}
      <p class="error">{t("settings.invites.error", { message: error })}</p>
    {/if}
    <ul>
      {#each invites as invite (invite.id)}
        <li>
          <span>{invite.role} — {status(invite)}</span>
          {#if !spent(invite)}
            <button type="button" onclick={() => revoke(invite.id)}>
              {t("settings.invites.revoke")}
            </button>
          {/if}
        </li>
      {/each}
      {#if invites.length === 0}<li class="empty">{t("settings.invites.empty")}</li>{/if}
    </ul>
    <h3>{t("settings.invites.members")}</h3>
    <!-- A redemption happens in the invitee's session, so nothing here can
         observe it: the GM re-reads on demand. -->
    <button type="button" onclick={() => refresh()}>{t("settings.invites.refresh")}</button>
    <ul>
      {#each members as member (member.user)}
        <li><span>{member.username} — {member.role}</span></li>
      {/each}
    </ul>
  </section>
{/if}

<style lang="scss">
  .invite-manager {
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
  .code {
    font-family: monospace;
  }
  .empty {
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
  input,
  select,
  button {
    min-height: 32px;
  }
</style>
