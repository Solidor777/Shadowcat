<script lang="ts">
  import type { TlsStatus } from "@shadowcat/types";
  import { t } from "@shadowcat/ui-kit";
  import {
    dismiss,
    httpsTargets,
    isDismissed,
    shouldShowHttpsNotice,
    type LocationLike,
  } from "@shadowcat/core";

  let {
    tls,
    loc = window.location,
    storage = readStorage(),
  }: {
    /** The server's published TLS status (`GET /api/config`). */
    tls: TlsStatus;
    /** The page location; injectable for tests. */
    loc?: LocationLike;
    /** Where a dismissal is remembered; `null` when the platform has no usable storage. */
    storage?: Pick<Storage, "getItem" | "setItem"> | null;
  } = $props();

  /**
   * The page's `localStorage`, or `null` when reading the property itself throws (some privacy
   * modes and sandboxed frames).
   * @returns The storage object or `null`.
   * @example
   * ```
   * // module-private; not part of the public API — the default for the `storage` prop
   * const storage = readStorage();
   * ```
   */
  function readStorage(): Storage | null {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  }

  let hidden = $state(false);
  const visible = $derived(
    !hidden && !isDismissed(storage, loc.origin) && shouldShowHttpsNotice(tls, loc),
  );
  const targets = $derived(httpsTargets(tls, loc));

  /**
   * Hide the notice and remember that for this origin.
   * @example
   * ```
   * // module-private; not part of the public API — bound to the dismiss button
   * onDismiss();
   * ```
   */
  function onDismiss() {
    hidden = true;
    dismiss(storage, loc.origin);
  }
</script>

{#if visible}
  <aside class="https-notice" role="note" data-testid="https-notice">
    <p>{t("entry.httpsNotice.body")}</p>
    <ul>
      {#each targets as url (url)}
        <li><a href={url} data-testid="https-notice-link">{url}</a></li>
      {/each}
    </ul>
    {#if tls.source === "self-signed"}
      <p class="fine-print">{t("entry.httpsNotice.selfSigned")}</p>
    {/if}
    <button type="button" onclick={onDismiss} data-testid="https-notice-dismiss">
      {t("entry.httpsNotice.dismiss")}
    </button>
  </aside>
{/if}

<style lang="scss">
  .https-notice {
    // Normal flow beneath the form, so on a short phone it scrolls with the page instead of
    // covering the login controls; the margin keeps it off the viewport edge.
    max-width: min(22rem, calc(100% - 2 * var(--space-4)));
    margin: var(--space-4) auto var(--space-4);
    display: grid;
    gap: var(--space-2);
    padding: var(--space-4);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    font-size: 0.875rem;
  }
  ul {
    margin: 0;
    padding-left: var(--space-4);
    overflow-wrap: anywhere;
  }
  // Touch target: each link is at least 2.75rem tall.
  li a {
    display: inline-flex;
    align-items: center;
    min-height: 2.75rem;
  }
  .fine-print {
    color: var(--text-muted);
  }
  button {
    justify-self: start;
    min-height: 2.75rem;
    min-width: 2.75rem;
  }
</style>
