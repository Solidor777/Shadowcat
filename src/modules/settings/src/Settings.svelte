<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { i18n, locale } from "@shadowcat/ui-kit";
  import { BUILTIN_THEMES, theme } from "@shadowcat/ui-kit";
  import ModuleManager from "./ModuleManager.svelte";
  import InviteManager from "./InviteManager.svelte";
  import UserManager from "./UserManager.svelte";

  const { role, t, leaveWorld, logout } = getAppContext();
  /**
   * Logs the current user out entirely — distinct from `leaveWorld` (the
   * button above this one): that only tears down the WS session and
   * navigates back to the worlds list (`leaveWorld`),
   * without touching the HTTP session, while `logout()` posts to
   * `/api/logout` to end it.
   * @returns Resolves once `logout()`'s own request completes; it does not
   *   surface a failed request to this caller (see `logout()`'s own doc).
   * @example
   * ```
   * // private function; not part of the public API — wired to the "Log out" button
   * await doLogout();
   * ```
   */
  async function doLogout() {
    await logout();
  }
</script>

<section class="panel">
  <h2>{t("settings.title")}</h2>
  <p>{t("settings.role", { role })}</p>
  <label>{t("settings.language")}
    <select value={locale()} onchange={(e) => i18n.setLocale(e.currentTarget.value)}>
      {#each i18n.locales as loc (loc)}<option value={loc}>{loc}</option>{/each}
    </select>
  </label>
  <label>{t("settings.theme.label")}
    <select value={theme.active} onchange={(e) => theme.setActive(e.currentTarget.value)}>
      {#each BUILTIN_THEMES as builtin (builtin.id)}
        <option value={builtin.id}>{t(builtin.labelKey)}</option>
      {/each}
      {#each Object.entries(theme.customThemes) as [id, custom] (id)}
        <option value={`custom:${id}`}>{custom.label}</option>
      {/each}
    </select>
  </label>
  {#if role === "gm"}
    <ModuleManager />
  {/if}
  <!-- Each self-gates: InviteManager on the world GM role, UserManager on the
       server admin tier. Both gates are advisory; the server re-checks. -->
  <InviteManager />
  <UserManager />
  <button onclick={leaveWorld}>{t("settings.leaveWorld")}</button>
  <button onclick={doLogout}>{t("settings.logout")}</button>
</section>

<style lang="scss">
  .panel {
    padding: var(--space-4);
    display: grid;
    gap: var(--space-3);
  }
  .panel p {
    color: var(--text-muted);
    margin: 0;
  }
</style>
