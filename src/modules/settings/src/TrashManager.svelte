<script lang="ts">
  import { getAppContext, confirmDialog } from "@shadowcat/ui-kit";
  import {
    listTrash,
    emptyTrash,
    deleteTrashEntry,
    fmtBytes,
    TrashEntryGoneError,
    type TrashEntry,
  } from "@shadowcat/core";

  // Server-administration surface: the trash lists server-side deleted files and lets an admin
  // remove them for good. Every route is gated on ServerRole::Admin server-side; rendering is
  // gated on `serverRole` (never `role`), which is advisory only.
  const { t, plural, serverRole } = getAppContext();

  /** Entries requested per page; matches the server's maximum page size. */
  const PAGE_SIZE = 200;

  let entries = $state<TrashEntry[]>([]);
  let loaded = $state(false);
  /** Whether the last page came back full, so another page may exist. */
  let hasMore = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let deleteError = $state<string | null>(null);
  let emptyError = $state<string | null>(null);
  /** The summary of the last "Empty trash", shown until the next action. */
  let emptied = $state<{
    /** Entries removed. */
    entries: number;
    /** Bytes freed. */
    bytes: number;
  } | null>(null);

  /**
   * Loads the first page of the trash, replacing the list. Reports failure on `error`, leaving
   * the prior list in place.
   * @returns Resolves once `entries`/`hasMore`/`error`/`loaded` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked at mount (admin only) and after each action
   * await load();
   * ```
   */
  async function load(): Promise<void> {
    error = null;
    try {
      const page = await listTrash({ limit: PAGE_SIZE });
      entries = page;
      hasMore = page.length === PAGE_SIZE;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loaded = true;
    }
  }
  if (serverRole === "admin") void load();

  /**
   * Appends the page that follows the last listed entry.
   * @returns Resolves once the page is appended or `error` reports why not; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "Load more" button
   * await loadMore();
   * ```
   */
  async function loadMore(): Promise<void> {
    const last = entries[entries.length - 1];
    if (!last) return;
    busy = true;
    error = null;
    try {
      const page = await listTrash({ limit: PAGE_SIZE, before: last.id });
      entries = [...entries, ...page];
      hasMore = page.length === PAGE_SIZE;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  /**
   * Permanently removes one entry after an explicit confirm dialog, then reloads the list. An
   * entry that is already gone (another admin or the retention sweep removed it) is not an
   * error: the list just refreshes. Declining the dialog changes no state.
   * @param entry The row to remove; only `id`/`original_path` are read.
   * @returns Resolves once `deleteError`/`entries` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's delete button
   * declare const entry: TrashEntry;
   * await removeOne(entry);
   * ```
   */
  async function removeOne(entry: TrashEntry): Promise<void> {
    const confirmed = await confirmDialog(
      { t },
      {
        titleKey: "settings.trash.deleteTitle",
        messageKey: "settings.trash.deleteConfirm",
        messageParams: { path: entry.original_path },
        confirmLabelKey: "settings.trash.deleteOne",
        cancelLabelKey: "settings.trash.cancel",
        danger: true,
      },
    );
    if (!confirmed) return;
    busy = true;
    deleteError = null;
    emptied = null;
    try {
      await deleteTrashEntry(entry.id);
    } catch (e) {
      if (!(e instanceof TrashEntryGoneError)) {
        deleteError = e instanceof Error ? e.message : String(e);
      }
    }
    await load();
    busy = false;
  }

  /**
   * Permanently removes every entry after an explicit confirm dialog that names how many entries
   * are listed (a lower bound while more pages exist), then reloads the list and
   * shows how much was removed. Declining the dialog changes no state.
   * @returns Resolves once `emptied`/`emptyError`/`entries` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "Empty trash" button
   * await emptyAll();
   * ```
   */
  async function emptyAll(): Promise<void> {
    const confirmed = await confirmDialog(
      { t },
      {
        titleKey: "settings.trash.emptyTitle",
        // The list is paginated: a full last page means the trash may hold more than the
        // loaded entries, so the count is then stated as a lower bound.
        messageKey: hasMore ? "settings.trash.emptyConfirmMore" : "settings.trash.emptyConfirm",
        messageParams: { n: entries.length },
        confirmLabelKey: "settings.trash.emptyAll",
        cancelLabelKey: "settings.trash.cancel",
        danger: true,
      },
    );
    if (!confirmed) return;
    busy = true;
    emptyError = null;
    emptied = null;
    try {
      emptied = await emptyTrash();
    } catch (e) {
      emptyError = e instanceof Error ? e.message : String(e);
    }
    await load();
    busy = false;
  }
</script>

{#if serverRole === "admin"}
  <section class="trash-manager">
    <h3>{t("settings.trash.title")}</h3>
    {#if error}
      <p class="error">{t("settings.trash.error", { message: error })}</p>
    {/if}
    {#if deleteError}
      <p class="error">{t("settings.trash.deleteError", { message: deleteError })}</p>
    {/if}
    {#if emptyError}
      <p class="error">{t("settings.trash.emptyError", { message: emptyError })}</p>
    {/if}
    {#if emptied}
      <p class="ok">
        {plural("settings.trash.emptyResult", emptied.entries, {
          n: emptied.entries,
          bytes: fmtBytes(emptied.bytes),
        })}
      </p>
    {/if}
    {#if !loaded}
      <p>{t("settings.trash.loading")}</p>
    {:else if entries.length === 0}
      <p>{t("settings.trash.empty")}</p>
    {:else}
      <button class="danger-outline" onclick={emptyAll} disabled={busy}>
        {t("settings.trash.emptyAll")}
      </button>
      <ul>
        {#each entries as entry (entry.id)}
          <li>
            <span class="item">
              <span class="path">{entry.original_path}</span>
              <span class="meta">
                {entry.kind === "dir" ? t("settings.trash.itemKindDir") : t("settings.trash.itemKindFile")}
                · {fmtBytes(entry.size_bytes)}
                · {t("settings.trash.trashedAt", { when: new Date(entry.trashed_at_ms).toLocaleString() })}
              </span>
            </span>
            <button class="danger-outline" onclick={() => removeOne(entry)} disabled={busy}>
              {t("settings.trash.deleteOne")}
            </button>
          </li>
        {/each}
      </ul>
      {#if hasMore}
        <button onclick={loadMore} disabled={busy}>{t("settings.trash.loadMore")}</button>
      {/if}
    {/if}
  </section>
{/if}

<style lang="scss">
  .trash-manager {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
  }
  .error {
    color: var(--danger);
  }
  ul {
    list-style: none;
    padding: 0;
    display: grid;
    gap: var(--space-2);
    justify-self: stretch;
  }
  li {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }
  .item {
    display: grid;
    min-width: 0;
  }
  .path {
    overflow-wrap: anywhere;
  }
  .meta {
    color: var(--text-muted);
  }
  button.danger-outline {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
  }
  button {
    min-height: 32px;
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
