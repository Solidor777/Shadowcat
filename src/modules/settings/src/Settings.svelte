<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { i18n, locale } from "@shadowcat/ui-kit";
  import { BUILTIN_THEMES, theme } from "@shadowcat/ui-kit";
  import ModuleManager from "./ModuleManager.svelte";
  import InviteManager from "./InviteManager.svelte";
  import UserManager from "./UserManager.svelte";
  import ThemeEditor from "./ThemeEditor.svelte";

  const { role, t, leaveWorld, logout, contributions } = getAppContext();

  // Bridges the framework-neutral registry's subscribe/snapshot to Svelte's reactivity —
  // the same pattern `Surface` uses, inlined here (rather than reusing `<Surface>`)
  // because each section also needs its own `settingsSection.labelKey` heading, which
  // `Surface` does not render.
  const subscribeSections = createSubscriber((update) => {
    const off = contributions.subscribe(update);
    return () => off();
  });
  const sections = $derived.by(() => {
    subscribeSections();
    return contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
  });

  /** Which theme the editor is open on, when it is open. */
  interface EditingTarget {
    /** The custom theme id being edited, or `null` for a new theme. Read
     * once at mount — the editor remounts when the edit target changes. */
    id: string | null;
  }
  /** The open editor's target, or `null` when the editor is closed. */
  let editing = $state<EditingTarget | null>(null);

  /**
   * Deletes a saved custom theme after an explicit confirm. Declining the
   * confirm is a true no-op. When the deleted theme was active, the controller
   * itself falls back to the default theme.
   * @param id The custom theme id to delete.
   * @param label The theme's display label, for the confirm message.
   * @example
   * ```
   * // private function; not part of the public API — wired to each custom theme row's delete button
   * removeCustomTheme("mine", "My Theme");
   * ```
   */
  function removeCustomTheme(id: string, label: string): void {
    if (!window.confirm(t("settings.theme.editor.deleteConfirm", { label }))) return;
    theme.deleteCustom(id);
  }

  /**
   * Closes the theme editor (the editor has already saved or discarded its
   * draft through the controller).
   * @example
   * ```
   * // private function; not part of the public API — wired to ThemeEditor's onclose
   * closeEditor();
   * ```
   */
  function closeEditor(): void {
    editing = null;
  }

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
  <div class="custom-themes">
    {#each Object.entries(theme.customThemes) as [id, custom] (id)}
      <div class="custom-theme-row">
        <span>{custom.label}</span>
        <button onclick={() => (editing = { id })}>{t("settings.theme.editor.edit")}</button>
        <button class="danger-outline" onclick={() => removeCustomTheme(id, custom.label)}>
          {t("settings.theme.editor.delete")}
        </button>
      </div>
    {/each}
    <button onclick={() => (editing = { id: null })}>{t("settings.theme.editor.new")}</button>
  </div>
  {#if editing !== null}
    <!-- Keyed remount: the editor reads its target theme once at mount. -->
    {#key editing.id}
      <ThemeEditor themeId={editing.id} onclose={closeEditor} />
    {/key}
  {/if}
  {#if role === "gm"}
    <ModuleManager />
  {/if}
  {#each sections as section (section.id)}
    {@const SectionComponent = section.component as Component<Record<string, unknown>>}
    <section class="contributed-settings-section">
      <h3>{t(section.settingsSection?.labelKey ?? "")}</h3>
      <SectionComponent {...(section.props ?? {})} />
    </section>
  {/each}
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
  .custom-themes {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
  }
  .contributed-settings-section {
    display: grid;
    gap: var(--space-2);
  }
  .contributed-settings-section h3 {
    margin: 0;
  }
  .custom-theme-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  button.danger-outline {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
