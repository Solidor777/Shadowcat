<script lang="ts">
  import { onMount } from "svelte";
  import { httpsTargets } from "@shadowcat/core";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { DuckSourcesController } from "./controller";
  import type { MicVadDenialReason } from "./micVad";
  import { readDuckingMirror, writeDuckingMirror, type DuckingPreferences } from "./duckingMirror";
  import type { OsMonitorStatus } from "./osMonitor";

  let {
    controller,
  }: {
    /** Owns the running `KeySource`/`OsMonitorSource` for this world session (constructed
     * once in `register(ctx)`, shared across every mount/unmount of this section). Its
     * `micToggle` field enables/disables the mic source for real; `null` until
     * `DuckingRuntime` has wired it (the mic toggle still persists the preference either
     * way). */
    controller: DuckSourcesController;
  } = $props();

  const appContext = getAppContext();
  const { t, audio } = appContext;

  /**
   * The message for a microphone denial. An `unavailable` microphone on a page that is not a
   * secure context has one remedy when the server also answers HTTPS: the address that does.
   * Without TLS on the server the only remedy is loopback, which the generic message names.
   * @param reason The denial `MicVadSource.enable` returned.
   * @returns The player-facing text.
   * @example
   * ```
   * // module-private; not part of the public API — rendered under the mic toggle
   * const text = denialText("unavailable");
   * ```
   */
  function denialText(reason: MicVadDenialReason): string {
    // `tls` is read at use time: it is reactive context state, not a value fixed at mount. The
    // https address is offered only from a plain-http page (from an https page the microphone is
    // already available, so the denial has another cause) and only when there is one to offer.
    const tls = appContext.tls;
    if (reason === "unavailable" && tls?.enabled && window.location.protocol === "http:") {
      const url = httpsTargets(tls, window.location)[0];
      if (url !== undefined) return t("ducking.micSource.denied.unavailableHttpsAvailable", { url });
    }
    return t(`ducking.micSource.denied.${reason}`);
  }

  let depth = $state(audio.duck.depth);

  let prefs = $state<DuckingPreferences>(readDuckingMirror(localStorage));
  let osStatus = $state<OsMonitorStatus>(controller.osMonitor.getStatus());
  let micDenial = $state<MicVadDenialReason | null>(null);
  let watchListText = $state(prefs.watchList.join(", "));

  onMount(() => {
    controller.applyPreferences(prefs);
    return controller.osMonitor.onStatusChange((s) => (osStatus = s));
  });

  /**
   * Persists the current `prefs` snapshot and re-applies it to the controller's sources.
   * @example
   * ```
   * // private function; not part of the public API — called after every field mutation
   * persist();
   * ```
   */
  function persist(): void {
    writeDuckingMirror(localStorage, prefs);
    controller.applyPreferences(prefs);
  }

  /**
   * Toggles the mic source: persists the preference immediately, then (once `onMicToggle`
   * exists) awaits the real enable/disable call and surfaces a denial reason: a denial shows
   * the reason and leaves the source off.
   * @param enabled The requested enabled state.
   * @example
   * ```
   * // private function; not part of the public API — wired to the mic checkbox
   * await toggleMic(true);
   * ```
   */
  async function toggleMic(enabled: boolean): Promise<void> {
    prefs.micEnabled = enabled;
    persist();
    if (controller.micToggle) {
      micDenial = (await controller.micToggle(enabled)) ?? null;
      if (micDenial) {
        prefs.micEnabled = false;
        persist();
      }
    }
  }

  /**
   * Parses the comma-separated watch-list text into the persisted array. A LIVE
   * editor — `persist()` -> `controller.applyPreferences` -> `OsMonitorSource.setWatch`
   * sends the `watch` frame immediately.
   * @example
   * ```
   * // private function; not part of the public API — wired to the watch-list input's onchange
   * commitWatchList();
   * ```
   */
  function commitWatchList(): void {
    prefs.watchList = watchListText.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
    persist();
  }

  const commandLine = $derived(
    `shadowcat audio-monitor --port ${prefs.osPort} --allow-origin ${typeof window !== "undefined" ? window.location.origin : ""} --watch ${prefs.watchList.join(",")}`,
  );
</script>

<div class="ducking-settings">
  <label>
    <input
      type="checkbox"
      checked={prefs.masterEnabled}
      onchange={(e) => { prefs.masterEnabled = e.currentTarget.checked; persist(); }}
    />
    {t("ducking.masterEnable")}
  </label>

  <fieldset disabled={!prefs.masterEnabled}>
    <legend>{t("ducking.keySource.title")}</legend>
    <label>
      <input
        type="checkbox"
        checked={prefs.keyEnabled}
        onchange={(e) => { prefs.keyEnabled = e.currentTarget.checked; persist(); }}
      />
      {t("ducking.keySource.enable")}
    </label>
  </fieldset>

  <fieldset disabled={!prefs.masterEnabled}>
    <legend>{t("ducking.micSource.title")}</legend>
    <label>
      <input type="checkbox" checked={prefs.micEnabled} onchange={(e) => toggleMic(e.currentTarget.checked)} />
      {t("ducking.micSource.enable")}
    </label>
    <label>
      {t("ducking.micSource.sensitivity")}
      <input
        type="range"
        min="0.5"
        max="6"
        step="0.1"
        value={prefs.micSensitivity}
        oninput={(e) => {
          prefs.micSensitivity = Number(e.currentTarget.value);
          persist();
          controller.micSetSensitivity?.(prefs.micSensitivity);
        }}
      />
    </label>
    <p class="denial" role="alert" aria-live="assertive">
      {#if micDenial}{denialText(micDenial)}{/if}
    </p>
  </fieldset>

  <fieldset disabled={!prefs.masterEnabled}>
    <legend>{t("ducking.osSource.title")}</legend>
    <label>
      <input
        type="checkbox"
        checked={prefs.osEnabled}
        onchange={(e) => { prefs.osEnabled = e.currentTarget.checked; persist(); }}
      />
      {t("ducking.osSource.enable")}
    </label>
    <label>
      {t("ducking.osSource.port")}
      <input
        type="number"
        value={prefs.osPort}
        onchange={(e) => { prefs.osPort = Number(e.currentTarget.value); controller.osMonitor.setPort(prefs.osPort); persist(); }}
      />
    </label>
    <label>
      {t("ducking.osSource.threshold")}
      <input
        type="range"
        min="0"
        max="0.2"
        step="0.005"
        value={prefs.osThreshold}
        oninput={(e) => { prefs.osThreshold = Number(e.currentTarget.value); persist(); }}
      />
    </label>
    <label>
      {t("ducking.osSource.watchList")}
      <input
        type="text"
        value={watchListText}
        oninput={(e) => (watchListText = e.currentTarget.value)}
        onchange={commitWatchList}
      />
    </label>
    <p aria-live="polite">{t(`ducking.osSource.status.${osStatus}`)}</p>
    {#if osStatus !== "connected" && osStatus !== "unsupported" && window.location.protocol === "https:"}
      <p class="hint">{t("ducking.osSource.httpsHint")}</p>
    {/if}
    <p class="command-line">{t("ducking.osSource.commandLine")}: <code>{commandLine}</code></p>
  </fieldset>

  <label>
    {t("ducking.depth")}
    <input
      type="range"
      min="0"
      max="1"
      step="0.05"
      value={depth}
      oninput={(e) => { depth = Number(e.currentTarget.value); audio.duck.setDepth(depth); }}
    />
  </label>
</div>

<style lang="scss">
  .ducking-settings {
    display: grid;
    gap: var(--space-3);
  }
  fieldset {
    display: grid;
    gap: var(--space-2);
  }
  .denial {
    color: var(--danger);
  }
  .hint {
    color: var(--text-muted);
    font-size: 0.875rem;
  }
  .command-line code {
    user-select: all;
  }
</style>
