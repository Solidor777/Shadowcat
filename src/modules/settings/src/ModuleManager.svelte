<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    listInstalledModules,
    getEnabledModules,
    setEnabledModules,
    type InstalledModuleInfo,
  } from "@shadowcat/core";

  const { world, t } = getAppContext();

  let installed = $state<InstalledModuleInfo[]>([]);
  let enabled = $state<Set<string>>(new Set());
  let loaded = $state(false);
  let saving = $state(false);
  let error = $state<string | null>(null);

  function manifestId(info: InstalledModuleInfo): string {
    const id = (info.manifest as { id?: unknown }).id;
    return typeof id === "string" ? id : "(unknown)";
  }

  async function load(): Promise<void> {
    error = null;
    try {
      const [inst, en] = await Promise.all([listInstalledModules(), getEnabledModules(world)]);
      installed = inst;
      enabled = new Set(en);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }
  load();

  function toggle(id: string): void {
    const next = new Set(enabled);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    enabled = next;
  }

  async function save(): Promise<void> {
    saving = true;
    error = null;
    try {
      await setEnabledModules(world, [...enabled]);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<section class="module-manager">
  <h3>{t("settings.modules.title")}</h3>
  {#if !loaded}
    <p>{t("settings.modules.loading")}</p>
  {:else if installed.length === 0}
    <p>{t("settings.modules.empty")}</p>
  {:else}
    <ul>
      {#each installed as info (manifestId(info))}
        <li>
          <label>
            <input
              type="checkbox"
              aria-label={manifestId(info)}
              checked={enabled.has(manifestId(info))}
              onchange={() => toggle(manifestId(info))}
            />
            {manifestId(info)}
          </label>
        </li>
      {/each}
    </ul>
    <button onclick={save} disabled={saving}>{t("settings.modules.save")}</button>
  {/if}
  {#if error}
    <p class="error">{t("settings.modules.error", { message: error })}</p>
  {/if}
</section>

<style lang="scss">
  .module-manager {
    display: grid;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
</style>
