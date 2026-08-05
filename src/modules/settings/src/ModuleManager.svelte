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

  /**
   * Display-only label: the manifest's author-declared `id`, falling back to
   * the install-folder id if the manifest omits one. Never used as a
   * toggle/save key — every call site in this file (`checked`/`onchange`
   * below, and `save()`'s `[...enabled]`) keys on `info.id` (the install
   * folder id, server-guaranteed present) instead, because the server's
   * enabled-module set is keyed on THAT id, which the manifest's own
   * declared id may legitimately differ from or collide with another
   * module's.
   * @param info The installed module to label.
   * @returns The manifest's declared id if it is a string, else `info.id`.
   * @example
   * ```
   * // private function; not part of the public API — used for the checkbox's
   * // label/aria-label below
   * displayName(info);
   * ```
   */
  function displayName(info: InstalledModuleInfo): string {
    const id = (info.manifest as { id?: unknown }).id;
    return typeof id === "string" ? id : info.id;
  }

  /**
   * Loads the installed-module catalog and this world's enabled set.
   * `listInstalledModules()` and `getEnabledModules(world)` are two
   * independent reads — like `InviteManager.refresh`'s pair — but this uses
   * `Promise.all`, not `allSettled`: if EITHER rejects, the destructuring
   * below never runs, so `installed` AND `enabled` both keep their empty
   * initial values even when the other read would have succeeded. `error`
   * is still set (the `catch`) and `loaded` still becomes `true` (the
   * `finally`), so the template's error paragraph renders below regardless —
   * this is a visible failure, not a silent one — but the checkbox list and
   * Save button fall back to the same "no modules installed" empty state
   * (`installed.length === 0` in the markup below) a genuinely empty world
   * would show, distinguished only by that separate error line.
   *
   * FINDING (reachability bounded; not fixed — this is a comment-only task):
   * this shares the affordance-loss shape `InviteManager.refresh`'s
   * `Promise.allSettled` choice documents guarding against — a GM who could
   * still see and toggle modules from a successful `listInstalledModules()`
   * loses that list because the unrelated `getEnabledModules(world)` read
   * failed, or vice versa. Both are ordinary independent network calls and
   * can fail without one implying the other did. Unlike `InviteManager`,
   * nothing here applies the succeeding half.
   * @returns Resolves once `installed`/`enabled`/`error`/`loaded` reflect
   *   the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked once at mount
   * await load();
   * ```
   */
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

  /**
   * Flips `id`'s LOCAL membership in `enabled` (not yet persisted — `save()`
   * sends the whole set). Assigns a new `Set` rather than mutating in place
   * so the `$state` reassignment is what triggers reactivity.
   * @param id The module's install-folder id (`info.id`) — the same key
   *   space `save()` and the server's enabled-module set use.
   * @returns Nothing; reassigns `enabled` as a side effect.
   * @example
   * ```
   * // private function; not part of the public API — wired to each checkbox's onchange
   * toggle(info.id);
   * ```
   */
  function toggle(id: string): void {
    const next = new Set(enabled);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    enabled = next;
  }

  /**
   * Persists the local `enabled` set as this world's ENTIRE enabled-module
   * set (`setEnabledModules(world, [...enabled])` — a whole-set replace, not
   * a diff). There is no optimistic-concurrency pre-image and no merge: the
   * server-side write is a plain settings overwrite
   * (`set_world_enabled_modules`, `src/server/src/data/sqlite.rs:1139-1146`,
   * calls `set_setting` with no read-then-check-then-write guard). Two GMs
   * saving concurrently is last-write-wins — the second save silently
   * clobbers whatever the first enabled, including modules the second GM
   * never saw toggled off.
   * @returns Resolves once `saving`/`error` reflect the outcome; never
   *   rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Save button
   * await save();
   * ```
   */
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
      {#each installed as info (info.id)}
        <li>
          <label>
            <input
              type="checkbox"
              aria-label={displayName(info)}
              checked={enabled.has(info.id)}
              onchange={() => toggle(info.id)}
            />
            {displayName(info)}
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
  input[type="checkbox"] {
    min-width: 36px;
    min-height: 36px;
  }
  button {
    min-height: 32px;
  }
</style>
