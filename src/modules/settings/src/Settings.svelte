<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { i18n, locale } from "@shadowcat/ui-kit";
  import ModuleManager from "./ModuleManager.svelte";
  import MemberManager from "./MemberManager.svelte";
  import UserManager from "./UserManager.svelte";

  const { role, t, leaveWorld, logout } = getAppContext();
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
  {#if role === "gm"}
    <ModuleManager />
  {/if}
  <!-- Each self-gates: MemberManager on the world GM role, UserManager on the
       server admin tier. Both gates are advisory; the server re-checks. -->
  <MemberManager />
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
