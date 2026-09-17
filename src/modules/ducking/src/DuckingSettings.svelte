<script lang="ts">
  import { onMount, onDestroy } from "svelte";
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

  const { t, audio } = getAppContext();

  let depth = $state(audio.duck.depth);

  let prefs = $state<DuckingPreferences>(readDuckingMirror(localStorage));
  let osStatus = $state<OsMonitorStatus>(controller.osMonitor.getStatus());
  let micDenial = $state<MicVadDenialReason | null>(null);
  let capturingKey = $state(false);
  let watchListText = $state(prefs.watchList.join(", "));

  /** The in-progress key-capture listener, tracked so `onDestroy`/`cancelKeyCapture` can
   * remove it even if no keydown ever fires — `startKeyCapture`'s own inline listener had no
   * way to reach it from outside the closure that added it. */
  let activeKeyCaptureListener: ((e: KeyboardEvent) => void) | null = null;

  onMount(() => {
    controller.applyPreferences(prefs);
    return controller.osMonitor.onStatusChange((s) => (osStatus = s));
  });

  onDestroy(() => {
    // A capture started then abandoned by navigating away, without this, leaves a capturing
    // `keydown` listener on `window` forever, hijacking the next keydown anywhere in the app.
    if (activeKeyCaptureListener) {
      window.removeEventListener("keydown", activeKeyCaptureListener, true);
      activeKeyCaptureListener = null;
    }
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
   * Starts listening for the next keydown and binds it as the push-to-duck key.
   * @example
   * ```
   * // private function; not part of the public API — wired to the "bind" button
   * startKeyCapture();
   * ```
   */
  function startKeyCapture(): void {
    capturingKey = true;
    const onKeydown = (e: KeyboardEvent) => {
      e.preventDefault();
      window.removeEventListener("keydown", onKeydown, true);
      activeKeyCaptureListener = null;
      capturingKey = false;
      if (e.code === "Escape") return; // cancels the capture rather than binding as the key
      prefs.keyBinding = e.code;
      persist();
    };
    activeKeyCaptureListener = onKeydown;
    window.addEventListener("keydown", onKeydown, true);
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
    <button type="button" onclick={startKeyCapture} disabled={capturingKey}>
      {capturingKey ? t("ducking.keySource.bindPrompt") : t("ducking.keySource.bind", { key: prefs.keyBinding })}
    </button>
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
      {#if micDenial}{t(`ducking.micSource.denied.${micDenial}`)}{/if}
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
  .command-line code {
    user-select: all;
  }
</style>
