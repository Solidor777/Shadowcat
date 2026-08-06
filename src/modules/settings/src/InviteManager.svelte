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

  /**
   * Re-reads this GM's invite list and the world roster from the server. Not
   * `Promise.all`: `listWorldInvites`/`listWorldMembers` are independent
   * reads, and letting a failed roster read reject the pair would blank the
   * invite list along with it — losing the revoke buttons (an AFFORDANCE, not
   * just data) because an unrelated request failed. `Promise.allSettled`
   * applies each result independently — `invites`/`members` update only for
   * whichever read actually succeeded — and still reports the first
   * rejection via `error`, so a partial failure neither hides the surviving
   * half nor goes unreported.
   *
   * The roster must come from here, not from `AppContext.members`
   * (`ctx.members`): that `SvelteMap` is refreshed only on a WS (re)connect
   * Welcome (`WorldSession`'s `#onWelcome`), not
   * on each individual join — a seat added while this session's connection
   * stays open does not reach it until the next reconnect, and minting or
   * redeeming an invite does not itself trigger one. This surface needs the
   * live count on demand, not the last-reconnect snapshot.
   * @returns Resolves once `invites`/`members`/`error` reflect the outcome;
   *   never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked after mint/revoke
   * // and from the manual "refresh" button
   * await refresh();
   * ```
   */
  async function refresh(): Promise<void> {
    const [gotInvites, gotMembers] = await Promise.allSettled([
      listWorldInvites(world),
      listWorldMembers(world),
    ]);
    if (gotInvites.status === "fulfilled") invites = gotInvites.value;
    if (gotMembers.status === "fulfilled") members = gotMembers.value;
    const failed = [gotInvites, gotMembers].find((r) => r.status === "rejected");
    if (failed) {
      const reason = (failed as PromiseRejectedResult).reason;
      error = reason instanceof Error ? reason.message : String(reason);
    }
  }
  if (role === "gm") void refresh();

  /**
   * Mints a single-use invite for `worldRole`, then refreshes the invite
   * list. `minted` is set from the response's `code` — the only point in
   * this component's life the plaintext code exists (see the `minted`
   * declaration above: only its hash is persisted).
   * @param e The form's submit event; only `preventDefault` is used.
   * @returns Resolves once `minted`/`error`/`invites` reflect the outcome;
   *   never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the mint form's onsubmit
   * await mint(submitEvent);
   * ```
   */
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

  /**
   * Revokes invite `id`, then refreshes the invite list so its revoke button
   * disappears (`spent()` becomes `true` for it).
   * @param id The invite's `id` — not its code, which is never sent back to
   *   the server after minting.
   * @returns Resolves once `invites`/`error` reflect the outcome; never
   *   rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each invite row's revoke button
   * await revoke(inviteId);
   * ```
   */
  async function revoke(id: string): Promise<void> {
    error = null;
    try {
      await revokeWorldInvite(world, id);
      await refresh();
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    }
  }

  /**
   * This invite's display status. Precedence — consumed, then revoked, then
   * expired, then active — matches the only order a real invite's fields can
   * be set in (a consumed or revoked row cannot un-consume/un-revoke).
   * Expiry here is a CLIENT-CLOCK GUESS (`Date.now()` against `expires_at`):
   * the server is the actual authority at redemption time, checking
   * `expires_at` against its OWN clock in one guarded UPDATE
   * (`SqliteRepository::consume_invite`), so a clock-skewed client can
   * label a row "active" moments before the server would refuse it, or
   * "expired" moments before the server would still accept it.
   *
   * Deliberately disagrees with `spent()` about what "expired" means: an
   * expired-but-unspent row is `"expired"` here yet `spent(i) === false`, so
   * its revoke button still renders in the template below
   * (`{#if !spent(invite)}`) — revoking an already-unusable code is harmless
   * and lets the GM prune it rather than wait on the server's own clock.
   * @param i The invite entry to describe.
   * @returns An i18n key naming this invite's current status.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the invite list's status span
   * status(invite);
   * ```
   */
  function status(i: InviteEntry): string {
    if (i.consumed_at !== null) return t("settings.invites.consumed");
    if (i.revoked_at !== null) return t("settings.invites.revoked");
    if (i.expires_at <= Date.now()) return t("settings.invites.expired");
    return t("settings.invites.active");
  }

  /**
   * Whether invite `i` can no longer be redeemed BY DESIGN — consumed or
   * revoked. Deliberately does NOT count expiry (see `status()`): the
   * template still renders a revoke button for an expired-but-unspent row,
   * letting a GM explicitly clear it instead of waiting on a redemption
   * attempt the server would reject anyway.
   * @param i The invite entry to check.
   * @returns `true` once consumed or revoked; `false` otherwise, including
   *   when merely expired.
   * @example
   * ```
   * // private function; not part of the public API — gates the revoke button in the invite list
   * spent(invite);
   * ```
   */
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
