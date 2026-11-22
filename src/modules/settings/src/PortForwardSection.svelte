<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { getPortMapStatus, setPortForward, type PortMapStatus } from "@shadowcat/core";

  // Server-administration surface: both routes are gated on ServerRole::Admin server-side;
  // rendering is gated on `serverRole` (never `role`), which is advisory only. The status carries
  // the host's external address, so nothing is fetched for a non-admin. Mounted through the
  // settings-section contract, so the host supplies the heading.
  const { t, serverRole } = getAppContext();

  let status = $state<PortMapStatus | null>(null);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);

  /**
   * Reads the status from the server, replacing it. A failure is reported on `error`, leaving the
   * prior status.
   * @returns Resolves once `status`/`error`/`loaded` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — invoked at mount (admin only) and by Refresh
   * await load();
   * ```
   */
  async function load(): Promise<void> {
    error = null;
    try {
      status = await getPortMapStatus();
    } catch (e) {
      error = t("settings.portForward.error", { message: e instanceof Error ? e.message : String(e) });
    } finally {
      loaded = true;
    }
  }
  if (serverRole === "admin") void load();

  /**
   * Stores the toggle and applies it at once, then shows the status the server returns.
   * @param enabled The admin's choice.
   * @returns Resolves once `status`/`error` reflect the outcome; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the toggle's change event
   * await toggle(true);
   * ```
   */
  async function toggle(enabled: boolean): Promise<void> {
    busy = true;
    error = null;
    try {
      status = await setPortForward(enabled);
    } catch (e) {
      error = t("settings.portForward.setError", {
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      busy = false;
    }
  }
</script>

{#if serverRole === "admin"}
  <div class="port-forward">
    {#if !loaded}
      <p>{t("settings.portForward.loading")}</p>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
    {#if status}
      <label class="toggle">
        <input
          type="checkbox"
          checked={status.enabled}
          disabled={busy || status.locked_by_config}
          onchange={(e) => toggle(e.currentTarget.checked)}
        />
        {t("settings.portForward.toggle")}
      </label>
      <p class="hint">{t("settings.portForward.warning")}</p>
      {#if status.locked_by_config}
        <p class="hint">{t("settings.portForward.locked")}</p>
      {/if}
      <p class="state" role="status">
        {t(`settings.portForward.state.${status.state}`)}
      </p>
      {#if status.detail}
        <p class="hint">{status.detail}</p>
      {/if}
      {#if status.protocol}
        <p class="meta">{t(`settings.portForward.protocol.${status.protocol}`)}</p>
      {/if}
      {#if status.external_address}
        <p class="meta">
          {t("settings.portForward.address", {
            address: status.external_address,
            port: status.external_port ?? "",
          })}
        </p>
      {/if}
      {#if status.lease_expires_in_secs !== null}
        <p class="meta">
          {t("settings.portForward.lease", { minutes: Math.ceil(status.lease_expires_in_secs / 60) })}
        </p>
      {/if}
      <button type="button" onclick={load} disabled={busy}>
        {t("settings.portForward.refresh")}
      </button>
    {/if}
  </div>
{/if}

<style lang="scss">
  .port-forward {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
  }
  .error {
    color: var(--danger);
  }
  .hint,
  .meta {
    color: var(--text-muted);
  }
  p {
    margin: 0;
  }
  .toggle {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  button {
    min-height: 32px;
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
      min-width: var(--input-height-coarse);
    }
    .toggle input {
      min-height: var(--input-height-coarse);
      min-width: var(--input-height-coarse);
    }
  }
</style>
