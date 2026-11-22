<script lang="ts">
  import { getAppContext, confirmDialog } from "@shadowcat/ui-kit";
  import {
    adminBackup,
    listBackups,
    deleteBackup,
    restoreBackup,
    pruneBackups,
    dismissRestoreFailed,
    fmtBytes,
    type BackupEntryView,
    type BackupListingView,
  } from "@shadowcat/core";

  // Server-administration surface: every route is gated on ServerRole::Admin server-side;
  // rendering is gated on `serverRole` (never `role`), which is advisory only. Mounted through the
  // settings-section contract, so the host supplies the heading.
  const { t, plural, serverRole, notify, notifyProgress } = getAppContext();

  let listing = $state<BackupListingView | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  /** Whether a manual server backup is in flight. */
  let backingUp = $state(false);
  /** The last "Apply now" result: how many backups it moved into the trash. */
  let pruned = $state<number | null>(null);
  /** The staged restore the server is stopping for, once the restore request succeeded. */
  let staged = $state<{
    /** The backup the restore will apply, as a display date. */
    when: string;
    /** The safety backup the server took of its current state. */
    safetyId: string;
  } | null>(null);

  /**
   * Formats a Unix-millisecond timestamp for display in the viewer's locale.
   * @param ms Unix epoch milliseconds.
   * @returns The locale date and time string.
   * @example
   * ```
   * // private function; not part of the public API — labels each row and each confirm dialog
   * const label = when(1789000000000);
   * ```
   */
  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  /**
   * Loads the backup list, replacing it. A failure is reported on `error`, leaving the prior list.
   * @returns Resolves once `listing`/`error`/`loaded` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked at mount (admin only) and after each action
   * await load();
   * ```
   */
  async function load(): Promise<void> {
    error = null;
    try {
      listing = await listBackups();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }
  if (serverRole === "admin") void load();

  /**
   * Takes a manual whole-server backup. The request is a single call with no incremental signal, so
   * an indeterminate progress notification stays up until it settles; the outcome is reported
   * through the ordinary notification channel, then the list reloads.
   * @returns Resolves once the backup settled and the progress notification is dismissed; never
   *   rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Back up now button
   * await backUpNow();
   * ```
   */
  async function backUpNow(): Promise<void> {
    backingUp = true;
    const toast = notifyProgress(t("settings.backups.running"), { value: null, max: 0 });
    try {
      const manifest = await adminBackup();
      notify(t("settings.backups.done", { size: fmtBytes(manifest.db_bytes) }), "info");
    } catch (e) {
      notify(
        t("settings.backups.backupError", { message: e instanceof Error ? e.message : String(e) }),
        "error",
      );
    } finally {
      toast.dismiss();
      backingUp = false;
    }
    await load();
  }

  /**
   * Restores `entry` after an explicit confirm dialog that says the server stops. The server takes
   * a safety backup of its current state, writes the restore marker and begins a graceful stop; it
   * does not restart itself, so on success the view switches to the "start the server again"
   * banner. Declining the dialog changes no state.
   * @param entry The backup row to restore; `id` and `manifest.created_at_unix_ms` are read.
   * @returns Resolves once `staged`/`actionError` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's Restore button
   * declare const entry: BackupEntryView;
   * await restore(entry);
   * ```
   */
  async function restore(entry: BackupEntryView): Promise<void> {
    const label = when(entry.manifest.created_at_unix_ms);
    const confirmed = await confirmDialog(
      { t },
      {
        titleKey: "settings.backups.restoreTitle",
        messageKey: "settings.backups.restoreConfirm",
        messageParams: { when: label },
        confirmLabelKey: "settings.backups.restoreAndStop",
        cancelLabelKey: "settings.backups.cancel",
        danger: true,
      },
    );
    if (!confirmed) return;
    busy = true;
    actionError = null;
    pruned = null;
    try {
      const result = await restoreBackup(entry.id);
      staged = { when: label, safetyId: result.pre_restore_id };
      notify(
        t("settings.backups.restoreStaged", { when: label, id: result.pre_restore_id }),
        "info",
      );
    } catch (e) {
      actionError = t("settings.backups.restoreError", {
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      busy = false;
    }
  }

  /**
   * Moves `entry` into the server's trash after an explicit confirm dialog, then reloads the list.
   * Declining the dialog changes no state.
   * @param entry The backup row to delete; `id` and `manifest.created_at_unix_ms` are read.
   * @returns Resolves once `actionError`/`listing` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's Delete button
   * declare const entry: BackupEntryView;
   * await remove(entry);
   * ```
   */
  async function remove(entry: BackupEntryView): Promise<void> {
    const confirmed = await confirmDialog(
      { t },
      {
        titleKey: "settings.backups.deleteTitle",
        messageKey: "settings.backups.deleteConfirm",
        messageParams: { when: when(entry.manifest.created_at_unix_ms) },
        confirmLabelKey: "settings.backups.delete",
        cancelLabelKey: "settings.backups.cancel",
        danger: true,
      },
    );
    if (!confirmed) return;
    busy = true;
    actionError = null;
    pruned = null;
    try {
      await deleteBackup(entry.id);
    } catch (e) {
      actionError = t("settings.backups.deleteError", {
        message: e instanceof Error ? e.message : String(e),
      });
    }
    await load();
    busy = false;
  }

  /**
   * Asks the server to clear the failed-restore marker, then hides the banner. A failure is
   * reported on `actionError` and leaves the banner showing.
   * @returns Resolves once the banner state and `actionError` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the banner's Dismiss button
   * await dismissFailure();
   * ```
   */
  async function dismissFailure(): Promise<void> {
    actionError = null;
    try {
      await dismissRestoreFailed();
      if (listing) listing = { ...listing, restore_failed: null };
    } catch (e) {
      actionError = t("settings.backups.dismissError", {
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }

  /**
   * Applies the server's retention policy now and reports how many backups it moved into the
   * trash, then reloads the list.
   * @returns Resolves once `pruned`/`actionError`/`listing` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the Apply now button
   * await applyNow();
   * ```
   */
  async function applyNow(): Promise<void> {
    busy = true;
    actionError = null;
    pruned = null;
    try {
      pruned = (await pruneBackups()).removed.length;
    } catch (e) {
      actionError = t("settings.backups.pruneError", {
        message: e instanceof Error ? e.message : String(e),
      });
    }
    await load();
    busy = false;
  }
</script>

{#if serverRole === "admin"}
  <div class="backup-manager">
    {#if staged}
      <p class="stopping" role="status">
        {t("settings.backups.stopping", { when: staged.when, id: staged.safetyId })}
      </p>
    {/if}
    {#if listing?.restore_failed}
      <div class="restore-failed">
        <p class="error" role="alert">
          {listing.restore_failed.id
            ? t("settings.backups.restoreFailed", {
                id: listing.restore_failed.id,
                when: when(listing.restore_failed.requested_at_ms),
                error: listing.restore_failed.error,
              })
            : t("settings.backups.restoreFailedUnknown", { error: listing.restore_failed.error })}
        </p>
        <button type="button" onclick={dismissFailure}>
          {t("settings.backups.dismissFailed")}
        </button>
      </div>
    {/if}
    <button type="button" onclick={backUpNow} disabled={backingUp || busy || staged !== null}>
      {t("settings.backups.backupNow")}
    </button>
    {#if error}
      <p class="error">{t("settings.backups.error", { message: error })}</p>
    {/if}
    {#if actionError}
      <p class="error">{actionError}</p>
    {/if}
    {#if !loaded}
      <p>{t("settings.backups.loading")}</p>
    {:else if listing}
      <div class="policy">
        <p>
          {#if listing.policy.keep_last === null && listing.policy.keep_days === null}
            {t("settings.backups.policyKeepAll")}
          {:else}
            {t("settings.backups.policyLabel")}
            {#if listing.policy.keep_last !== null}
              {plural("settings.backups.policyLast", listing.policy.keep_last, {
                n: listing.policy.keep_last,
              })}
            {/if}
            {#if listing.policy.keep_days !== null}
              {plural("settings.backups.policyDays", listing.policy.keep_days, {
                n: listing.policy.keep_days,
              })}
            {/if}
          {/if}
        </p>
        <p class="hint">{t("settings.backups.policyHint")}</p>
        <button
          type="button"
          onclick={applyNow}
          disabled={busy || staged !== null ||
            (listing.policy.keep_last === null && listing.policy.keep_days === null)}
        >
          {t("settings.backups.applyNow")}
        </button>
        {#if pruned !== null}
          <p class="ok">
            {pruned === 0
              ? t("settings.backups.prunedNone")
              : plural("settings.backups.pruned", pruned, { n: pruned })}
          </p>
        {/if}
      </div>
      <p class="hint">{t("settings.backups.root", { root: listing.root })}</p>
      {#if listing.entries.length === 0}
        <p>{t("settings.backups.empty")}</p>
      {:else}
        <ul>
          {#each listing.entries as entry (entry.id)}
            {@const label = when(entry.manifest.created_at_unix_ms)}
            <li>
              <span class="item">
                <span class="when">{label}</span>
                <span class="meta">
                  {t(`settings.backups.kind.${entry.manifest.kind}`)}
                  · {fmtBytes(entry.total_bytes)}
                  · {plural("settings.backups.assetFiles", entry.manifest.asset_file_count, {
                    n: entry.manifest.asset_file_count,
                  })}
                  · {t("settings.backups.version", { version: entry.manifest.shadowcat_version })}
                </span>
                {#if entry.manifest.note}
                  <span class="meta">{entry.manifest.note}</span>
                {/if}
              </span>
              <span class="actions">
                <button
                  type="button"
                  aria-label={t("settings.backups.restoreRow", { when: label })}
                  onclick={() => restore(entry)}
                  disabled={busy || staged !== null}
                >
                  {t("settings.backups.restore")}
                </button>
                <button
                  type="button"
                  class="danger-outline"
                  aria-label={t("settings.backups.deleteRow", { when: label })}
                  onclick={() => remove(entry)}
                  disabled={busy || staged !== null}
                >
                  {t("settings.backups.delete")}
                </button>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
{/if}

<style lang="scss">
  .backup-manager {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
  }
  .error {
    color: var(--danger);
  }
  .restore-failed {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }
  .hint,
  .meta {
    color: var(--text-muted);
  }
  p {
    margin: 0;
  }
  .policy {
    display: grid;
    gap: var(--space-1);
    justify-items: start;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: var(--space-2);
    justify-self: stretch;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }
  .item {
    display: grid;
    min-width: 0;
    flex: 1 1 14rem;
  }
  .when {
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  button {
    min-height: 32px;
  }
  button.danger-outline {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
      min-width: var(--input-height-coarse);
    }
  }
</style>
