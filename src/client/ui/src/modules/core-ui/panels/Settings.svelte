<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  import { logout } from "../../../lib/api";
  import { navigate } from "../../../lib/route.svelte";
  import { i18n } from "../../../lib/i18n.svelte";

  const { role, t } = getAppContext();
  async function doLogout() {
    await logout();
    navigate({ name: "login" });
  }
</script>

<section class="panel">
  <h2>{t("settings.title")}</h2>
  <p>{t("settings.role", { role })}</p>
  <label>{t("settings.language")}
    <select value={i18n.locale} onchange={(e) => i18n.setLocale(e.currentTarget.value)}>
      {#each i18n.locales as loc (loc)}<option value={loc}>{loc}</option>{/each}
    </select>
  </label>
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
